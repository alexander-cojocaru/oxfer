use std::io::SeekFrom;
use std::path::Path;

use async_trait::async_trait;
use tokio::io::AsyncSeekExt;

use crate::{BoxRead, BoxWrite, Entry, EntryKind, Error, RemoteFs, RemotePath, Result};

/// The local file system exposed through [`RemoteFs`], so the transfer engine
/// can treat uploads and downloads uniformly.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFs;

fn map_err(e: std::io::Error, path: &RemotePath) -> Error {
    match e.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(path.clone()),
        std::io::ErrorKind::PermissionDenied => Error::PermissionDenied(path.clone()),
        std::io::ErrorKind::AlreadyExists => Error::AlreadyExists(path.clone()),
        _ => Error::Io(e),
    }
}

fn entry(path: RemotePath, m: &std::fs::Metadata) -> Entry {
    let t = m.file_type();
    let kind = if t.is_dir() {
        EntryKind::Dir
    } else if t.is_symlink() {
        EntryKind::Symlink
    } else if t.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        Some(m.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let permissions = None;
    Entry {
        path,
        kind,
        size: m.len(),
        modified: m.modified().ok(),
        permissions,
    }
}

#[async_trait]
impl RemoteFs for LocalFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>> {
        let mut rd = tokio::fs::read_dir(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        let mut out = Vec::new();
        while let Some(e) = rd.next_entry().await? {
            let name = e.file_name().to_string_lossy().into_owned();
            // symlink_metadata: report links as links, never follow them while listing
            let m = tokio::fs::symlink_metadata(e.path()).await?;
            out.push(entry(path.join(&name), &m));
        }
        Ok(out)
    }

    async fn stat(&self, path: &RemotePath) -> Result<Entry> {
        let m = tokio::fs::metadata(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(entry(path.clone(), &m))
    }

    async fn realpath(&self, path: &RemotePath) -> Result<RemotePath> {
        let p = tokio::fs::canonicalize(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(RemotePath::new(p.to_string_lossy().into_owned()))
    }

    async fn lstat(&self, path: &RemotePath) -> Result<Entry> {
        let m = tokio::fs::symlink_metadata(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(entry(path.clone(), &m))
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxRead> {
        let mut f = tokio::fs::File::open(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        if offset > 0 {
            f.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxWrite> {
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(offset == 0)
            .open(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        if offset > 0 {
            f.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(f))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        tokio::fs::create_dir(path.as_str())
            .await
            .map_err(|e| map_err(e, path))
    }

    async fn remove(&self, path: &RemotePath) -> Result<()> {
        let m = tokio::fs::symlink_metadata(path.as_str())
            .await
            .map_err(|e| map_err(e, path))?;
        let r = if m.is_dir() {
            tokio::fs::remove_dir(path.as_str()).await
        } else {
            tokio::fs::remove_file(path.as_str()).await
        };
        r.map_err(|e| map_err(e, path))
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        tokio::fs::rename(Path::new(from.as_str()), Path::new(to.as_str()))
            .await
            .map_err(|e| map_err(e, from))
    }
}
