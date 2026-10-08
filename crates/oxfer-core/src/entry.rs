use std::time::SystemTime;

use crate::RemotePath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Metadata for a single remote file system entry.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: RemotePath,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, if the protocol reports them.
    pub permissions: Option<u32>,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.file_name().unwrap_or("/")
    }

    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}
