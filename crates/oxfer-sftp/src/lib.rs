//! SFTP backend implementing [`oxfer_core::RemoteFs`].

mod client;
mod fs;

pub use client::{Auth, ConnectOptions, HostKeyDecision, HostKeyPrompt, HostKeyVerifier};
pub use fs::SftpFs;
