//! Protocol-agnostic traits, transfer engine and error types.

mod entry;
mod error;
mod fs;
mod fsutil;
mod local;
mod path;
pub mod transfer;

pub use entry::{Entry, EntryKind};
pub use error::{Error, Result};
pub use fs::{BoxRead, BoxWrite, RemoteFs, Shell, ShellControl};
pub use fsutil::{mkdir_all, remove_all};
pub use local::LocalFs;
pub use path::RemotePath;
