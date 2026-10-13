use async_trait::async_trait;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::{Entry, Error, RemotePath, Result};

pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// Input side of an interactive remote shell.
#[async_trait]
pub trait ShellControl: Send + Sync {
    /// Send keystrokes to the remote terminal.
    async fn write(&self, data: &[u8]) -> Result<()>;
    /// Tell the remote terminal its new size in character cells.
    async fn resize(&self, cols: u32, rows: u32) -> Result<()>;
    /// End the shell.
    async fn close(&self) -> Result<()>;
}

/// An interactive remote shell on a pseudo-terminal.
pub struct Shell {
    /// Terminal output; closed when the shell has ended.
    pub output: mpsc::Receiver<Vec<u8>>,
    pub control: Arc<dyn ShellControl>,
}

/// A connected remote file system. Implemented by each protocol backend.
#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>>;
    async fn stat(&self, path: &RemotePath) -> Result<Entry>;
    /// Canonical absolute form of `path` (resolves `.`, `..` and symlinks where
    /// the backend can). The default is a lexical normalisation.
    async fn realpath(&self, path: &RemotePath) -> Result<RemotePath> {
        Ok(path.normalize())
    }
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
    /// Start an interactive shell of `cols` x `rows` characters on the server.
    async fn open_shell(&self, _cols: u32, _rows: u32) -> Result<Shell> {
        Err(Error::Unsupported("remote shell"))
    }
}
