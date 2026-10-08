use std::io::SeekFrom;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use oxfer_core::{BoxRead, BoxWrite, Entry, EntryKind, Error, RemoteFs, RemotePath, Result};
use russh::client::{Handle, Handler};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::Metadata;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, OpenFlags, StatusCode};
use tokio::io::AsyncSeekExt;

pub struct SftpFs {
    sftp: SftpSession,
    // Keeps the SSH connection alive for as long as the SFTP session is used.
    _session: Box<dyn std::any::Any + Send + Sync>,
}

impl SftpFs {
    pub(crate) fn new<H: Handler + 'static>(sftp: SftpSession, session: Handle<H>) -> Self {
        Self {
            sftp,
            _session: Box::new(session),
        }
    }
}

fn map_err(e: SftpError, path: &RemotePath) -> Error {
    match &e {
        SftpError::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => Error::NotFound(path.clone()),
            StatusCode::PermissionDenied => Error::PermissionDenied(path.clone()),
            _ => Error::Protocol(e.to_string()),
        },
        SftpError::IO(_) => Error::Connection(e.to_string()),
        _ => Error::Protocol(e.to_string()),
    }
}

fn entry(path: RemotePath, m: &Metadata) -> Entry {
    let kind = match m.file_type() {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    };
    Entry {
        path,
        kind,
        size: m.size.unwrap_or(0),
        modified: m.mtime.map(|t| UNIX_EPOCH + Duration::from_secs(t.into())),
        permissions: m.permissions.map(|p| p & 0o7777),
    }
}

#[async_trait]
impl RemoteFs for SftpFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>> {
        let dir = self
            .sftp
            .read_dir(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(dir
            .map(|e| entry(path.join(&e.file_name()), &e.metadata()))
            .collect())
    }

    async fn stat(&self, path: &RemotePath) -> Result<Entry> {
        let m = self
            .sftp
            .metadata(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(entry(path.clone(), &m))
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxRead> {
        let mut f = self
            .sftp
            .open_with_flags(path.as_str(), OpenFlags::READ)
            .await
            .map_err(|e| map_err(e, path))?;
        if offset > 0 {
            f.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxWrite> {
        let mut flags = OpenFlags::WRITE | OpenFlags::CREATE;
        if offset == 0 {
            flags |= OpenFlags::TRUNCATE;
        }
        let mut f = self
            .sftp
            .open_with_flags(path.as_str(), flags)
            .await
            .map_err(|e| map_err(e, path))?;
        if offset > 0 {
            f.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.sftp
            .create_dir(path.as_str())
            .await
            .map_err(|e| map_err(e, path))
    }

    async fn remove(&self, path: &RemotePath) -> Result<()> {
        let m = self
            .sftp
            .symlink_metadata(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        let r = if m.file_type() == FileType::Dir {
            self.sftp.remove_dir(path.as_str()).await
        } else {
            self.sftp.remove_file(path.as_str()).await
        };
        r.map_err(|e| map_err(e, path))
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.sftp
            .rename(from.as_str(), to.as_str())
            .await
            .map_err(|e| map_err(e, from))
    }
}
