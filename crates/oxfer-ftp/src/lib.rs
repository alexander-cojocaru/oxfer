//! FTP/FTPS backend implementing [`oxfer_core::RemoteFs`].

mod client;
mod fs;
pub mod listing;
mod transfer;

pub use client::{FtpConnectOptions, TlsMode};
pub use fs::FtpFs;
