use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc::UnboundedSender, Semaphore};
use tokio::task::JoinSet;

use crate::{Entry, Error, RemoteFs, RemotePath, Result};

use super::{
    Conflict, ConflictAction, Control, Event, Job, OverwritePolicy, Plan, Summary, TransferOptions,
    PART_SUFFIX,
};

const CHUNK: usize = 256 * 1024;

/// Copies files from `src` to `dst`. Cheap to clone.
#[derive(Clone)]
pub struct Transfer {
    pub(super) src: Arc<dyn RemoteFs>,
    pub(super) dst: Arc<dyn RemoteFs>,
    pub(super) opts: TransferOptions,
}

enum Outcome {
    Done(u64),
    Skipped(String),
}

impl Transfer {
    pub fn new(src: Arc<dyn RemoteFs>, dst: Arc<dyn RemoteFs>, opts: TransferOptions) -> Self {
        Self { src, dst, opts }
    }

    /// Execute a plan. Never fails as a whole: per-file errors are collected in
    /// the returned [`Summary`] (and emitted as [`Event::Failed`]).
    pub async fn run(
        &self,
        plan: Plan,
        events: Option<UnboundedSender<Event>>,
        control: Control,
    ) -> Summary {
        let emit = move |ev: Event| {
            if let Some(tx) = &events {
                let _ = tx.send(ev);
            }
        };
        let emit = Arc::new(emit);
        emit(Event::Planned {
            files: plan.jobs.len(),
            bytes: plan.total_bytes(),
        });

        let mut summary = Summary {
            skipped: plan.skipped.clone(),
            ..Default::default()
        };

        for dir in &plan.dirs {
            if let Err(e) = self.ensure_dir(dir).await {
                summary.failed.push((dir.clone(), e));
            }
        }
        if !summary.failed.is_empty() {
            // Without the directory tree the file copies would only produce noise.
            return finish(summary, &*emit);
        }

        let sem = Arc::new(Semaphore::new(self.opts.concurrency.max(1)));
        let mut set = JoinSet::new();
        for job in plan.jobs {
            let this = self.clone();
            let sem = sem.clone();
            let control = control.clone();
            let emit = emit.clone();
            set.spawn(async move {
                let path = job.src.path.clone();
                let permit = tokio::select! {
                    p = sem.acquire_owned() => p.expect("semaphore is never closed"),
                    _ = control.token().cancelled() => return (path, Err(Error::Cancelled)),
                };
                let r = this.run_job(&job, &control, &*emit).await;
                drop(permit);
                (path, r)
            });
        }

        while let Some(joined) = set.join_next().await {
            let Ok((path, result)) = joined else { continue };
            match result {
                Ok(Outcome::Done(n)) => {
                    summary.completed += 1;
                    summary.bytes += n;
                }
                Ok(Outcome::Skipped(reason)) => summary.skipped.push((path, reason)),
                Err(Error::Cancelled) => summary.cancelled = true,
                Err(e) => summary.failed.push((path, e)),
            }
        }
        finish(summary, &*emit)
    }

    /// Plan and run in one go.
    pub async fn copy(
        &self,
        sources: &[RemotePath],
        dst_dir: &RemotePath,
        events: Option<UnboundedSender<Event>>,
        control: Control,
    ) -> Result<Summary> {
        let plan = self.plan(sources, dst_dir).await?;
        Ok(self.run(plan, events, control).await)
    }

    async fn ensure_dir(&self, dir: &RemotePath) -> Result<()> {
        match self.dst.stat(dir).await {
            Ok(e) if e.is_dir() => Ok(()),
            Ok(_) => Err(Error::AlreadyExists(dir.clone())),
            Err(Error::NotFound(_)) => match self.dst.mkdir(dir).await {
                // lost a race with another creator
                Err(Error::AlreadyExists(_)) => Ok(()),
                r => r,
            },
            Err(e) => Err(e),
        }
    }

    async fn run_job(
        &self,
        job: &Job,
        control: &Control,
        emit: &(dyn Fn(Event) + Send + Sync),
    ) -> Result<Outcome> {
        let id = job.id;
        emit(Event::Started {
            id,
            src: job.src.path.clone(),
            dst: job.dst.clone(),
            size: job.src.size,
        });
        let mut attempt = 0;
        let result = loop {
            match self.copy_file(job, control, emit).await {
                Err(e) if e.is_retriable() && attempt < self.opts.retries => {
                    attempt += 1;
                    emit(Event::Retrying {
                        id,
                        attempt,
                        error: e.to_string(),
                    });
                    let delay = self.opts.retry_delay * 2u32.saturating_pow(attempt - 1);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = control.token().cancelled() => break Err(Error::Cancelled),
                    }
                }
                r => break r,
            }
        };
        match &result {
            Ok(Outcome::Done(bytes)) => emit(Event::Finished { id, bytes: *bytes }),
            Ok(Outcome::Skipped(reason)) => emit(Event::Skipped {
                id,
                reason: reason.clone(),
            }),
            Err(Error::Cancelled) => {}
            Err(e) => emit(Event::Failed {
                id,
                error: e.to_string(),
            }),
        }
        result
    }

    async fn copy_file(
        &self,
        job: &Job,
        control: &Control,
        emit: &(dyn Fn(Event) + Send + Sync),
    ) -> Result<Outcome> {
        control.checkpoint().await?;
        let total = job.src.size;

        let existing = match self.dst.stat(&job.dst).await {
            Ok(e) if e.is_dir() => {
                return Err(Error::Invalid(format!("{} is a directory", job.dst)))
            }
            Ok(e) => Some(e),
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        if let Some(dst) = &existing {
            if let Some(reason) = self.resolve_conflict(&job.src, dst).await {
                return Ok(Outcome::Skipped(reason));
            }
        }

        let tmp = RemotePath::new(format!("{}{PART_SUFFIX}", job.dst));
        let mut offset = 0;
        if self.opts.resume {
            if let Ok(part) = self.dst.stat(&tmp).await {
                if part.size <= total {
                    offset = part.size;
                }
            }
        }

        let mut reader = self.src.open_read(&job.src.path, offset).await?;
        let mut writer = self.dst.open_write(&tmp, offset).await?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = offset;
        emit(Event::Progress {
            id: job.id,
            done,
            total,
        });
        loop {
            control.checkpoint().await?;
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).await?;
            done += n as u64;
            emit(Event::Progress {
                id: job.id,
                done,
                total,
            });
        }
        writer.shutdown().await?;
        drop(writer);
        drop(reader);

        let written = self.dst.stat(&tmp).await?.size;
        if written != total {
            return Err(Error::Protocol(format!(
                "size mismatch for {}: expected {total} bytes, destination has {written}",
                job.dst
            )));
        }
        if self.opts.verify {
            let (a, b) = tokio::try_join!(
                digest(&*self.src, &job.src.path, control),
                digest(&*self.dst, &tmp, control)
            )?;
            if a != b {
                // The partial content is wrong; make sure a retry starts over.
                let _ = self.dst.remove(&tmp).await;
                return Err(Error::Protocol(format!(
                    "checksum mismatch for {}",
                    job.dst
                )));
            }
        }

        if existing.is_some() {
            self.dst.remove(&job.dst).await?;
        }
        self.dst.rename(&tmp, &job.dst).await?;
        Ok(Outcome::Done(total))
    }

    /// `Some(reason)` means: skip this file.
    async fn resolve_conflict(&self, src: &Entry, dst: &Entry) -> Option<String> {
        match self.opts.overwrite {
            OverwritePolicy::Overwrite => None,
            OverwritePolicy::Skip => Some("already exists".into()),
            OverwritePolicy::NewerOnly => match (src.modified, dst.modified) {
                (Some(s), Some(d)) if s <= d => Some("destination is not older".into()),
                _ => None,
            },
            OverwritePolicy::Ask => {
                let action = match &self.opts.on_conflict {
                    Some(f) => {
                        f(Conflict {
                            src: src.clone(),
                            dst: dst.clone(),
                        })
                        .await
                    }
                    None => ConflictAction::Skip,
                };
                (action == ConflictAction::Skip).then(|| "skipped by user".into())
            }
        }
    }
}

async fn digest(fs: &dyn RemoteFs, path: &RemotePath, control: &Control) -> Result<[u8; 32]> {
    let mut r = fs.open_read(path, 0).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        control.checkpoint().await?;
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buf[..n]);
    }
}

fn finish(summary: Summary, emit: &(dyn Fn(Event) + Send + Sync)) -> Summary {
    emit(Event::Done {
        completed: summary.completed,
        skipped: summary.skipped.len(),
        failed: summary.failed.len(),
        bytes: summary.bytes,
        cancelled: summary.cancelled,
    });
    summary
}
