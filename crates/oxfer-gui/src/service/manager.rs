//! [`SessionManager`]: open sessions, file operations and running transfers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use oxfer_core::transfer::{Control, Job, Plan, Transfer};
use oxfer_core::{remove_all, LocalFs, RemoteFs, RemotePath};
use tokio_util::sync::CancellationToken;

use super::connect::{self, ConnectEnv, ConnectRequest};
use super::dto::{EntryDto, ProtocolDto, SessionInfo};
use super::error::{ServiceError, ServiceResult};
use super::profiles::ProfileStore;
use super::prompt::Prompter;
use super::transfers::{
    conflict_resolver, core_options, forward_events, EventSink, SkippedDto, TransferEvent,
    TransferRequest, TransferState,
};

/// Id of the built-in local file system session.
pub const LOCAL_SESSION: &str = "local";

struct Session {
    fs: Arc<dyn RemoteFs>,
    info: SessionInfo,
}

struct TransferEntry {
    request: TransferRequest,
    control: Control,
    cancel: CancellationToken,
    state: Arc<Mutex<TransferState>>,
}

/// Owns all connections and transfers. Has no UI dependencies: questions go
/// through a [`Prompter`], progress through an [`EventSink`].
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Session>>,
    next_session: AtomicU64,
    transfers: Mutex<HashMap<u64, TransferEntry>>,
    next_transfer: AtomicU64,
    sink: Arc<dyn EventSink>,
    profiles: ProfileStore,
    env: ConnectEnv,
}

fn local_home() -> String {
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    if cfg!(windows) {
        home.replace('\\', "/")
    } else {
        home
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_path(path: &str) -> ServiceResult<RemotePath> {
    if path.is_empty() || path.contains('\0') {
        return Err(ServiceError::invalid(format!("invalid path {path:?}")));
    }
    Ok(RemotePath::new(path))
}

impl SessionManager {
    pub fn new(profiles: ProfileStore, env: ConnectEnv, sink: Arc<dyn EventSink>) -> Arc<Self> {
        Self::with_local_home(profiles, env, sink, local_home())
    }

    /// Like [`new`](Self::new) but with an explicit start directory for the
    /// local session.
    pub fn with_local_home(
        profiles: ProfileStore,
        env: ConnectEnv,
        sink: Arc<dyn EventSink>,
        local_home: String,
    ) -> Arc<Self> {
        let mut sessions = HashMap::new();
        sessions.insert(
            LOCAL_SESSION.to_string(),
            Session {
                fs: Arc::new(LocalFs),
                info: SessionInfo {
                    id: LOCAL_SESSION.into(),
                    label: "Local".into(),
                    protocol: ProtocolDto::Local,
                    home: local_home,
                },
            },
        );
        Arc::new(Self {
            sessions: RwLock::new(sessions),
            next_session: AtomicU64::new(1),
            transfers: Mutex::new(HashMap::new()),
            next_transfer: AtomicU64::new(1),
            sink,
            profiles,
            env,
        })
    }

    pub fn profiles(&self) -> &ProfileStore {
        &self.profiles
    }

    /// Register an already connected file system (used by tests and embedders).
    pub fn add_session(
        &self,
        fs: Arc<dyn RemoteFs>,
        label: &str,
        protocol: ProtocolDto,
        home: &str,
    ) -> SessionInfo {
        let id = format!("s{}", self.next_session.fetch_add(1, Ordering::Relaxed));
        let info = SessionInfo {
            id: id.clone(),
            label: label.into(),
            protocol,
            home: home.into(),
        };
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Session {
                    fs,
                    info: info.clone(),
                },
            );
        info
    }

    pub fn sessions(&self) -> Vec<SessionInfo> {
        let mut v: Vec<_> = self
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.info.clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn fs(&self, session: &str) -> ServiceResult<Arc<dyn RemoteFs>> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .map(|s| s.fs.clone())
            .ok_or_else(|| ServiceError::unknown_session(session))
    }

    pub async fn connect(
        &self,
        req: ConnectRequest,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<SessionInfo> {
        let c = connect::connect(&self.env, &self.profiles, req, prompter).await?;
        let home = connect::start_dir(&*c.fs, c.start_dir).await;
        Ok(self.add_session(c.fs, &c.label, c.protocol, &home))
    }

    /// Close a session. Running transfers keep their own reference and finish.
    pub fn disconnect(&self, session: &str) -> ServiceResult<()> {
        if session == LOCAL_SESSION {
            return Err(ServiceError::invalid("the local session cannot be closed"));
        }
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session)
            .map(|_| ())
            .ok_or_else(|| ServiceError::unknown_session(session))
    }

    pub async fn list_dir(&self, session: &str, path: &str) -> ServiceResult<Vec<EntryDto>> {
        let fs = self.fs(session)?;
        let entries = fs.list(&check_path(path)?).await?;
        Ok(entries
            .iter()
            .filter(|e| !matches!(e.name(), "." | ".."))
            .map(EntryDto::from)
            .collect())
    }

    pub async fn stat(&self, session: &str, path: &str) -> ServiceResult<EntryDto> {
        let fs = self.fs(session)?;
        Ok((&fs.stat(&check_path(path)?).await?).into())
    }

    /// Canonical form of `path` (e.g. to resolve a typed-in path).
    pub async fn realpath(&self, session: &str, path: &str) -> ServiceResult<String> {
        let fs = self.fs(session)?;
        Ok(fs.realpath(&check_path(path)?).await?.to_string())
    }

    pub async fn mkdir(&self, session: &str, path: &str) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        Ok(fs.mkdir(&check_path(path)?).await?)
    }

    pub async fn rename(&self, session: &str, from: &str, to: &str) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        let (from, to) = (check_path(from)?, check_path(to)?);
        // Never silently replace an existing entry.
        match fs.lstat(&to).await {
            Ok(_) => return Err(oxfer_core::Error::AlreadyExists(to).into()),
            Err(oxfer_core::Error::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(fs.rename(&from, &to).await?)
    }

    /// Delete files and directories (recursively).
    pub async fn delete(&self, session: &str, paths: &[String]) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        for p in paths {
            let p = check_path(p)?;
            if p.parent().is_none() && p.is_absolute() {
                return Err(ServiceError::invalid(
                    "refusing to delete the root directory",
                ));
            }
            remove_all(&*fs, &p).await?;
        }
        Ok(())
    }

    /// Start copying `sources` into `dest_dir`. Returns the transfer id; all
    /// progress is reported through the [`EventSink`].
    pub async fn start_transfer(
        &self,
        req: TransferRequest,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<u64> {
        if req.sources.is_empty() {
            return Err(ServiceError::invalid("nothing to transfer"));
        }
        let dest = check_path(&req.dest_dir)?;
        let sources = req
            .sources
            .iter()
            .map(|s| check_path(s))
            .collect::<ServiceResult<Vec<_>>>()?;
        if req.src_session == req.dst_session {
            let d = dest.normalize();
            for s in &sources {
                let s = s.normalize();
                if s.parent().as_ref() == Some(&d) || d == s {
                    return Err(ServiceError::invalid(format!(
                        "{s} is already in {d}; source and destination are the same"
                    )));
                }
                if d.as_str()
                    .starts_with(&format!("{}/", s.as_str().trim_end_matches('/')))
                {
                    return Err(ServiceError::invalid(format!(
                        "cannot copy {s} into itself"
                    )));
                }
            }
        }
        self.spawn_transfer(req, None, None, prompter)
    }

    fn spawn_transfer(
        &self,
        req: TransferRequest,
        plan: Option<Plan>,
        retry_of: Option<u64>,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<u64> {
        let src = self.fs(&req.src_session)?;
        let dst = self.fs(&req.dst_session)?;
        let id = self.next_transfer.fetch_add(1, Ordering::Relaxed);
        let control = Control::new();
        let cancel = CancellationToken::new();
        let state = Arc::new(Mutex::new(TransferState::default()));
        lock(&self.transfers).insert(
            id,
            TransferEntry {
                request: req.clone(),
                control: control.clone(),
                cancel: cancel.clone(),
                state: state.clone(),
            },
        );
        self.sink.emit(TransferEvent::Queued {
            transfer_id: id,
            src_session: req.src_session.clone(),
            dst_session: req.dst_session.clone(),
            sources: req.sources.clone(),
            dest_dir: req.dest_dir.clone(),
            retry_of,
        });

        let opts = core_options(
            &req.options,
            conflict_resolver(id, prompter, cancel.clone()),
        );
        let transfer = Transfer::new(src, dst, opts);
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let plan = match plan {
                Some(p) => Ok(p),
                None => {
                    let sources: Vec<RemotePath> =
                        req.sources.iter().map(RemotePath::new).collect();
                    let dest = RemotePath::new(req.dest_dir.clone());
                    tokio::select! {
                        p = transfer.plan(&sources, &dest) => p,
                        _ = cancel.cancelled() => Err(oxfer_core::Error::Cancelled),
                    }
                }
            };
            let plan = match plan {
                Ok(p) => p,
                Err(e) => {
                    let cancelled = matches!(e, oxfer_core::Error::Cancelled);
                    {
                        let mut st = lock(&state);
                        st.done = true;
                        st.plan_failed = true;
                    }
                    if !cancelled {
                        sink.emit(TransferEvent::Failed {
                            transfer_id: id,
                            job_id: None,
                            path: req.dest_dir.clone(),
                            error: e.to_string(),
                        });
                    }
                    sink.emit(TransferEvent::Done {
                        transfer_id: id,
                        completed: 0,
                        skipped: 0,
                        failed: usize::from(!cancelled),
                        bytes: 0,
                        cancelled,
                    });
                    return;
                }
            };
            {
                let mut st = lock(&state);
                st.dirs = plan.dirs.clone();
                st.jobs = plan.jobs.clone();
                st.plan_skipped = plan
                    .skipped
                    .iter()
                    .map(|(p, r)| SkippedDto {
                        path: p.to_string(),
                        reason: r.clone(),
                    })
                    .collect();
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let fwd = tokio::spawn(forward_events(id, rx, sink, state));
            transfer.run(plan, Some(tx), control).await;
            let _ = fwd.await;
        });
        Ok(id)
    }

    fn with_transfer<T>(
        &self,
        id: u64,
        f: impl FnOnce(&TransferEntry) -> ServiceResult<T>,
    ) -> ServiceResult<T> {
        let map = lock(&self.transfers);
        let entry = map
            .get(&id)
            .ok_or_else(|| ServiceError::unknown_transfer(id))?;
        f(entry)
    }

    pub fn pause_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.control.pause();
            Ok(())
        })?;
        self.sink.emit(TransferEvent::Paused { transfer_id: id });
        Ok(())
    }

    pub fn resume_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.control.resume();
            Ok(())
        })?;
        self.sink.emit(TransferEvent::Resumed { transfer_id: id });
        Ok(())
    }

    pub fn cancel_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.cancel.cancel();
            t.control.cancel();
            Ok(())
        })
    }

    pub fn is_transfer_done(&self, id: u64) -> ServiceResult<bool> {
        self.with_transfer(id, |t| Ok(lock(&t.state).done))
    }

    /// Run the files of a finished transfer that did not complete (failed or
    /// cancelled) again, as a new transfer.
    pub async fn retry_transfer(&self, id: u64, prompter: Arc<dyn Prompter>) -> ServiceResult<u64> {
        let (req, plan) = self.with_transfer(id, |t| {
            let st = lock(&t.state);
            if !st.done {
                return Err(ServiceError::invalid("the transfer is still running"));
            }
            if st.plan_failed {
                return Ok((t.request.clone(), None));
            }
            let jobs: Vec<Job> = st
                .jobs
                .iter()
                .filter(|j| !st.settled.contains(&j.id))
                .enumerate()
                .map(|(i, j)| Job {
                    id: i,
                    src: j.src.clone(),
                    dst: j.dst.clone(),
                })
                .collect();
            if jobs.is_empty() {
                return Err(ServiceError::invalid("nothing to retry"));
            }
            let plan = Plan {
                dirs: st.dirs.clone(),
                jobs,
                skipped: vec![],
            };
            Ok((t.request.clone(), Some(plan)))
        })?;
        self.spawn_transfer(req, plan, Some(id), prompter)
    }

    /// Drop finished transfers from the registry.
    pub fn clear_finished(&self) {
        lock(&self.transfers).retain(|_, t| !lock(&t.state).done);
    }
}
