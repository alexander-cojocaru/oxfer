//! Transfer engine: plans and runs file copies between any two [`RemoteFs`]
//! (local ↔ SFTP, local ↔ FTP, ...), with recursion, globs, parallelism,
//! resume, retry with backoff, overwrite policies and optional verification.

mod control;
mod engine;
mod glob;
mod plan;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub use control::Control;
pub use engine::Transfer;
pub use plan::{Job, Plan};

use crate::{Entry, Error, RemotePath};

/// Suffix of the temporary file a transfer writes to before the final rename.
/// A leftover partial file is what makes resuming possible.
pub const PART_SUFFIX: &str = ".oxfer-part";

pub type JobId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwritePolicy {
    Overwrite,
    Skip,
    /// Overwrite only if the source is newer than the destination.
    NewerOnly,
    /// Consult [`TransferOptions::on_conflict`]; skips if none is set.
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub src: Entry,
    pub dst: Entry,
}

pub type ConflictResolver =
    Arc<dyn Fn(Conflict) -> Pin<Box<dyn Future<Output = ConflictAction> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct TransferOptions {
    pub recursive: bool,
    /// Expand `*`, `?`, `[..]` and `{..}` in the last component of each source.
    /// Turn off when sources are exact paths (e.g. picked from a file list).
    pub glob: bool,
    pub overwrite: OverwritePolicy,
    pub on_conflict: Option<ConflictResolver>,
    /// Continue a leftover partial file instead of starting over.
    pub resume: bool,
    /// Re-read both files after the transfer and compare SHA-256 digests.
    pub verify: bool,
    /// Number of files transferred in parallel.
    pub concurrency: usize,
    /// Retries per file for transient errors.
    pub retries: u32,
    /// First backoff delay; doubles with every attempt.
    pub retry_delay: Duration,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            glob: true,
            overwrite: OverwritePolicy::Overwrite,
            on_conflict: None,
            resume: true,
            verify: false,
            concurrency: 4,
            retries: 3,
            retry_delay: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Planned {
        files: usize,
        bytes: u64,
    },
    Started {
        id: JobId,
        src: RemotePath,
        dst: RemotePath,
        size: u64,
    },
    Progress {
        id: JobId,
        done: u64,
        total: u64,
    },
    Retrying {
        id: JobId,
        attempt: u32,
        error: String,
    },
    Skipped {
        id: JobId,
        reason: String,
    },
    Finished {
        id: JobId,
        bytes: u64,
    },
    Failed {
        id: JobId,
        error: String,
    },
    Done {
        completed: usize,
        skipped: usize,
        failed: usize,
        bytes: u64,
        cancelled: bool,
    },
}

#[derive(Debug, Default)]
pub struct Summary {
    pub completed: usize,
    pub bytes: u64,
    pub skipped: Vec<(RemotePath, String)>,
    pub failed: Vec<(RemotePath, Error)>,
    pub cancelled: bool,
}

impl Summary {
    pub fn is_success(&self) -> bool {
        self.failed.is_empty() && !self.cancelled
    }
}
