//! Profiles, `~/.ssh/config` resolution and credential storage for oxfer.
//!
//! `ProxyJump` (profile field or `~/.ssh/config`) becomes a chain of
//! [`oxfer_sftp::ConnectOptions::jump`] hops; jump hosts name hosts as
//! `[user@]host[:port]` or ssh_config aliases (profile names are not resolved).

mod config;
mod jump;
mod profile;
pub mod secrets;
pub mod ssh_config;

pub use config::Config;
pub use jump::jump_chain;
pub use profile::{AuthMethod, Profile, Protocol};
pub use secrets::{KeyringStore, MemoryStore, Secret, SecretStore};
pub use ssh_config::{ResolvedHost, SshConfig};

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config file {path}: {message}", path = .path.display())]
    Parse { path: PathBuf, message: String },
    #[error("cannot serialize config: {0}")]
    Serialize(String),
    #[error("invalid profile: {0}")]
    Invalid(String),
    #[error("profile '{0}' already exists")]
    Duplicate(String),
    #[error("profile '{0}' not found")]
    NotFound(String),
    #[error("no config directory available on this platform")]
    NoConfigDir,
    #[error("secret store: {0}")]
    Secret(String),
    #[error("no stored password for profile '{0}'")]
    MissingSecret(String),
    #[error("profile '{0}' needs interactive authentication; prompt the user")]
    InteractiveAuth(String),
    #[error("no user for profile '{0}' (set `user` or `User` in ssh config)")]
    MissingUser(String),
    #[error("protocol {0} cannot be used for SFTP options")]
    WrongProtocol(&'static str),
    #[error("invalid ProxyJump hop: {0}")]
    InvalidJump(String),
}
