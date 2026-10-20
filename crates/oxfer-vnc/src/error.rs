use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The server rejected the credentials.
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    /// The server needs a password and none was given.
    #[error("the server requires a password")]
    PasswordRequired,
    /// The server only offers an authentication that needs a user name.
    #[error("the server requires a user name")]
    UsernameRequired,
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("timed out connecting to the server")]
    Timeout,
}

impl Error {
    pub(crate) fn protocol(msg: impl Into<String>) -> Self {
        Error::Protocol(msg.into())
    }
}
