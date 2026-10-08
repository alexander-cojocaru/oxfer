//! Transfer DTOs, progress events and the event forwarding loop.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxfer_core::transfer::{
    Conflict, ConflictAction, ConflictResolver, Event, Job, JobId, OverwritePolicy, TransferOptions,
};
use oxfer_core::RemotePath;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

use super::prompt::{ConflictChoice, Prompt, PromptAnswer, Prompter};

/// How often coalesced progress updates are pushed to the UI.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverwriteDto {
    Overwrite,
    Skip,
    NewerOnly,
    #[default]
    Ask,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOptionsDto {
    #[serde(default)]
    pub overwrite: OverwriteDto,
    #[serde(default = "yes")]
    pub resume: bool,
    #[serde(default)]
    pub verify: bool,
    #[serde(default)]
    pub concurrency: Option<usize>,
}

impl Default for TransferOptionsDto {
    fn default() -> Self {
        Self {
            overwrite: OverwriteDto::Ask,
            resume: true,
            verify: false,
            concurrency: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub src_session: String,
    pub dst_session: String,
    pub sources: Vec<String>,
    pub dest_dir: String,
    #[serde(default)]
    pub options: TransferOptionsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDto {
    pub path: String,
    pub reason: String,
}

/// Progress notification pushed to the UI (`transfer-event`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransferEvent {
    Queued {
        transfer_id: u64,
        src_session: String,
        dst_session: String,
        sources: Vec<String>,
        dest_dir: String,
        /// Set when this transfer retries an earlier one.
        retry_of: Option<u64>,
    },
    Planned {
        transfer_id: u64,
        files: usize,
        bytes: u64,
        skipped: Vec<SkippedDto>,
    },
    Started {
        transfer_id: u64,
        job_id: JobId,
        src: String,
        dst: String,
        size: u64,
    },
    Progress {
        transfer_id: u64,
        job_id: JobId,
        done: u64,
        total: u64,
    },
    Retrying {
        transfer_id: u64,
        job_id: JobId,
        attempt: u32,
        error: String,
    },
    Skipped {
        transfer_id: u64,
        job_id: JobId,
        path: String,
        reason: String,
    },
    Finished {
        transfer_id: u64,
        job_id: JobId,
        bytes: u64,
    },
    Failed {
        transfer_id: u64,
        /// `None` when the transfer failed as a whole (e.g. while planning).
        job_id: Option<JobId>,
        path: String,
        error: String,
    },
    Paused {
        transfer_id: u64,
    },
    Resumed {
        transfer_id: u64,
    },
    Done {
        transfer_id: u64,
        completed: usize,
        skipped: usize,
        failed: usize,
        bytes: u64,
        cancelled: bool,
    },
}

/// Receives transfer events (the Tauri layer emits them to the window).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: TransferEvent);
}

impl<F: Fn(TransferEvent) + Send + Sync> EventSink for F {
    fn emit(&self, event: TransferEvent) {
        self(event)
    }
}

/// Bookkeeping for one transfer, used for retries.
#[derive(Default)]
pub(crate) struct TransferState {
    pub dirs: Vec<RemotePath>,
    pub jobs: Vec<Job>,
    pub plan_skipped: Vec<SkippedDto>,
    /// Jobs that need no retry (finished or skipped).
    pub settled: HashSet<JobId>,
    pub done: bool,
    /// Planning failed: a retry has to plan again.
    pub plan_failed: bool,
}

pub(crate) fn core_options(
    dto: &TransferOptionsDto,
    resolver: ConflictResolver,
) -> TransferOptions {
    TransferOptions {
        recursive: true,
        overwrite: match dto.overwrite {
            OverwriteDto::Overwrite => OverwritePolicy::Overwrite,
            OverwriteDto::Skip => OverwritePolicy::Skip,
            OverwriteDto::NewerOnly => OverwritePolicy::NewerOnly,
            OverwriteDto::Ask => OverwritePolicy::Ask,
        },
        on_conflict: (dto.overwrite == OverwriteDto::Ask).then_some(resolver),
        resume: dto.resume,
        verify: dto.verify,
        concurrency: dto.concurrency.unwrap_or(4).clamp(1, 16),
        // Sources are exact paths picked in the file list, never patterns.
        glob: false,
        ..TransferOptions::default()
    }
}

/// Conflict resolver asking through the prompter, one question at a time,
/// honouring "apply to all". Cancelling the transfer answers "skip".
pub(crate) fn conflict_resolver(
    transfer_id: u64,
    prompter: Arc<dyn Prompter>,
    cancel: CancellationToken,
) -> ConflictResolver {
    let sticky: Arc<Mutex<Option<ConflictAction>>> = Arc::default();
    let serial = Arc::new(tokio::sync::Mutex::new(()));
    Arc::new(move |c: Conflict| {
        let (prompter, sticky, serial, cancel) = (
            prompter.clone(),
            sticky.clone(),
            serial.clone(),
            cancel.clone(),
        );
        Box::pin(async move {
            let _one_at_a_time = serial.lock().await;
            if let Some(a) = *sticky.lock().unwrap_or_else(|e| e.into_inner()) {
                return a;
            }
            let prompt = Prompt::Conflict {
                transfer_id,
                src: (&c.src).into(),
                dst: (&c.dst).into(),
            };
            let answer = tokio::select! {
                a = prompter.ask(prompt) => a,
                _ = cancel.cancelled() => return ConflictAction::Skip,
            };
            match answer {
                PromptAnswer::Conflict {
                    action,
                    apply_to_all,
                } => {
                    let a = match action {
                        ConflictChoice::Overwrite => ConflictAction::Overwrite,
                        ConflictChoice::Skip => ConflictAction::Skip,
                    };
                    if apply_to_all {
                        *sticky.lock().unwrap_or_else(|e| e.into_inner()) = Some(a);
                    }
                    a
                }
                _ => ConflictAction::Skip,
            }
        })
    })
}

fn job_of(ev: &Event) -> Option<JobId> {
    match ev {
        Event::Started { id, .. }
        | Event::Progress { id, .. }
        | Event::Retrying { id, .. }
        | Event::Skipped { id, .. }
        | Event::Finished { id, .. }
        | Event::Failed { id, .. } => Some(*id),
        Event::Planned { .. } | Event::Done { .. } => None,
    }
}

/// Map engine events to DTOs, coalescing progress updates to at most one
/// per job every [`PROGRESS_INTERVAL`].
pub(crate) async fn forward_events(
    transfer_id: u64,
    mut rx: UnboundedReceiver<Event>,
    sink: Arc<dyn EventSink>,
    state: Arc<Mutex<TransferState>>,
) {
    let mut pending: BTreeMap<JobId, (u64, u64)> = BTreeMap::new();
    let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let flush_one = |id: JobId, (done, total): (u64, u64)| {
        sink.emit(TransferEvent::Progress {
            transfer_id,
            job_id: id,
            done,
            total,
        })
    };
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                if let Event::Progress { id, done, total } = ev {
                    pending.insert(id, (done, total));
                    continue;
                }
                if let Some(id) = job_of(&ev) {
                    if let Some(p) = pending.remove(&id) {
                        flush_one(id, p);
                    }
                }
                if matches!(ev, Event::Done { .. }) {
                    for (id, p) in std::mem::take(&mut pending) {
                        flush_one(id, p);
                    }
                }
                if let Some(dto) = map_event(transfer_id, ev, &state) {
                    sink.emit(dto);
                }
            }
            _ = tick.tick() => {
                for (id, p) in std::mem::take(&mut pending) {
                    flush_one(id, p);
                }
            }
        }
    }
    for (id, p) in pending {
        flush_one(id, p);
    }
}

fn map_event(transfer_id: u64, ev: Event, state: &Mutex<TransferState>) -> Option<TransferEvent> {
    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
    let path_of = |st: &TransferState, id: JobId| {
        st.jobs
            .get(id)
            .map(|j| j.src.path.to_string())
            .unwrap_or_default()
    };
    Some(match ev {
        Event::Planned { files, bytes } => TransferEvent::Planned {
            transfer_id,
            files,
            bytes,
            skipped: st.plan_skipped.clone(),
        },
        Event::Started { id, src, dst, size } => TransferEvent::Started {
            transfer_id,
            job_id: id,
            src: src.to_string(),
            dst: dst.to_string(),
            size,
        },
        Event::Progress { .. } => return None,
        Event::Retrying { id, attempt, error } => TransferEvent::Retrying {
            transfer_id,
            job_id: id,
            attempt,
            error,
        },
        Event::Skipped { id, reason } => {
            st.settled.insert(id);
            TransferEvent::Skipped {
                transfer_id,
                job_id: id,
                path: path_of(&st, id),
                reason,
            }
        }
        Event::Finished { id, bytes } => {
            st.settled.insert(id);
            TransferEvent::Finished {
                transfer_id,
                job_id: id,
                bytes,
            }
        }
        Event::Failed { id, error } => TransferEvent::Failed {
            transfer_id,
            job_id: Some(id),
            path: path_of(&st, id),
            error,
        },
        Event::Done {
            completed,
            skipped,
            failed,
            bytes,
            cancelled,
        } => {
            st.done = true;
            TransferEvent::Done {
                transfer_id,
                completed,
                skipped,
                failed,
                bytes,
                cancelled,
            }
        }
    })
}
