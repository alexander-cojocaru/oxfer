//! Protocol-agnostic traits, transfer engine and error types.

mod entry;
mod error;
mod fs;
mod local;
mod path;
pub mod transfer;

pub use entry::{Entry, EntryKind};
pub use error::{Error, Result};
pub use fs::{BoxRead, BoxWrite, RemoteFs};
pub use local::LocalFs;
pub use path::RemotePath;
