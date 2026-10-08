use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Entry, RemotePath, Result};

pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// A connected remote file system. Implemented by each protocol backend.
#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>>;
    async fn stat(&self, path: &RemotePath) -> Result<Entry>;
    /// Like [`stat`](Self::stat) but does not follow a final symlink.
    async fn lstat(&self, path: &RemotePath) -> Result<Entry> {
        self.stat(path).await
    }
    /// Open a file for reading, starting at byte `offset`.
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxRead>;
    /// Open a file for writing, starting at byte `offset` (0 truncates/creates).
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxWrite>;
    async fn mkdir(&self, path: &RemotePath) -> Result<()>;
    /// Remove a file or an empty directory.
    async fn remove(&self, path: &RemotePath) -> Result<()>;
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()>;
}
