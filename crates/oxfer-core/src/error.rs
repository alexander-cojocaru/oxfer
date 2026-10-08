use crate::RemotePath;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(RemotePath),
    #[error("permission denied: {0}")]
    PermissionDenied(RemotePath),
    #[error("already exists: {0}")]
    AlreadyExists(RemotePath),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("host key verification failed: {0}")]
    HostKey(String),
    #[error("connection error: {0}")]
    Connection(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("operation not supported: {0}")]
    Unsupported(&'static str),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
