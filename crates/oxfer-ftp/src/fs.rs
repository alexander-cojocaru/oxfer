use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use oxfer_core::{BoxRead, BoxWrite, Entry, EntryKind, Error, RemoteFs, RemotePath, Result};
use suppaftp::tokio::AsyncRustlsFtpStream;
use suppaftp::FtpError;
use tokio::sync::Mutex;

use crate::listing::{parse_list, parse_mlsd, parse_mlst};
use crate::transfer::{FtpReader, FtpWriter};

pub(crate) struct State {
    pub(crate) ftp: AsyncRustlsFtpStream,
    /// The server advertised MLST/MLSD; cleared if it turns out not to work.
    pub(crate) mlst: bool,
}

/// FTP / FTPS backend.
///
/// FTP has one transfer per control connection, so every operation takes a
/// mutex; an open `BoxRead`/`BoxWrite` holds it until the transfer is
/// finished or dropped (see the `transfer` module). A broken control
/// connection is not re-established: later calls fail with
/// [`Error::Connection`] and the caller should connect again.
pub struct FtpFs {
    state: Arc<Mutex<State>>,
}

/// What a bare `550` (no explanatory text) means for the operation at hand.
#[derive(Clone, Copy)]
pub(crate) enum Plain {
    NotFound,
    Denied,
}

impl FtpFs {
    pub(crate) fn from_state(state: State) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }
}

fn reply_code(e: &FtpError) -> Option<u32> {
    match e {
        FtpError::UnexpectedResponse(r) => Some(r.status.code()),
        _ => None,
    }
}

pub(crate) fn map_err(e: FtpError, path: &RemotePath, plain: Plain) -> Error {
    let FtpError::UnexpectedResponse(r) = &e else {
        return match e {
            FtpError::ConnectionError(e) => Error::Connection(e.to_string()),
            FtpError::SecureError(e) => Error::Connection(e),
            e => Error::Protocol(e.to_string()),
        };
    };
    let text = String::from_utf8_lossy(&r.body).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    match r.status.code() {
        530 | 331 | 332 | 532 => Error::Auth(text.trim().to_string()),
        421 => Error::Connection(text.trim().to_string()),
        550 | 553 | 450 | 451 => {
            if has(&[
                "no such",
                "not found",
                "not exist",
                "doesn't exist",
                "cannot find",
            ]) {
                Error::NotFound(path.clone())
            } else if has(&["permission", "denied", "not allowed", "forbidden", "access"]) {
                Error::PermissionDenied(path.clone())
            } else if has(&["exists", "already"]) {
                Error::AlreadyExists(path.clone())
            } else {
                match (r.status.code(), plain) {
                    (553, _) | (_, Plain::Denied) => Error::PermissionDenied(path.clone()),
                    _ => Error::NotFound(path.clone()),
                }
            }
        }
        _ => Error::Protocol(e.to_string()),
    }
}

fn nf(e: FtpError, path: &RemotePath) -> Error {
    map_err(e, path, Plain::NotFound)
}

/// 500/502/504: "command not understood / implemented".
fn is_unsupported(e: &FtpError) -> bool {
    matches!(reply_code(e), Some(500 | 502 | 504))
}

impl State {
    async fn list(&mut self, path: &RemotePath) -> Result<Vec<Entry>> {
        if self.mlst {
            match self.ftp.mlsd(Some(path.as_str())).await {
                Ok(lines) => {
                    return Ok(parse_mlsd(&lines)
                        .into_iter()
                        .map(|e| {
                            let p = path.join(&e.name);
                            e.into_entry(p)
                        })
                        .collect())
                }
                Err(e) if is_unsupported(&e) => {
                    tracing::debug!("MLSD not supported, falling back to LIST");
                    self.mlst = false;
                }
                Err(e) => return Err(nf(e, path)),
            }
        }
        let lines = self
            .ftp
            .list(Some(path.as_str()))
            .await
            .map_err(|e| nf(e, path))?;
        Ok(parse_list(&lines, SystemTime::now())
            .into_iter()
            .map(|e| {
                let p = path.join(&e.name);
                e.into_entry(p)
            })
            .collect())
    }

    async fn stat(&mut self, path: &RemotePath) -> Result<Entry> {
        let Some(name) = path.file_name() else {
            return Ok(Entry {
                path: path.clone(),
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
                permissions: None,
            });
        };
        if self.mlst {
            match self.ftp.mlst(Some(path.as_str())).await {
                Ok(line) => {
                    if let Some(e) = parse_mlst(&line) {
                        return Ok(e.into_entry(path.clone()));
                    }
                }
                Err(e) if is_unsupported(&e) => self.mlst = false,
                Err(e) => return Err(nf(e, path)),
            }
        }
        // Fall back to listing the parent directory.
        let parent = path.parent().unwrap_or_else(RemotePath::root);
        self.list(&parent)
            .await?
            .into_iter()
            .find(|e| e.name() == name)
            .ok_or_else(|| Error::NotFound(path.clone()))
    }

    async fn start_read(&mut self, path: &RemotePath, offset: u64) -> Result<TransferStreamT> {
        if offset > 0 {
            self.ftp
                .resume_transfer(offset as usize)
                .await
                .map_err(|e| match reply_code(&e) {
                    Some(500 | 501 | 502 | 504) => Error::Unsupported("resuming transfers (REST)"),
                    _ => nf(e, path),
                })?;
        }
        match self.ftp.retr_as_stream(path.as_str()).await {
            Ok(s) => Ok(s),
            Err(e) => {
                self.clear_rest(offset).await;
                Err(nf(e, path))
            }
        }
    }

    async fn start_write(&mut self, path: &RemotePath, offset: u64) -> Result<TransferStreamT> {
        let denied = |e| map_err(e, path, Plain::Denied);
        if offset == 0 {
            return self
                .ftp
                .put_with_stream(path.as_str())
                .await
                .map_err(denied);
        }
        // APPE is universally supported; use it when we are exactly at EOF.
        let size = self
            .ftp
            .size(path.as_str())
            .await
            .map_err(|e| nf(e, path))? as u64;
        if size == offset {
            return self
                .ftp
                .append_with_stream(path.as_str())
                .await
                .map_err(denied);
        }
        if size < offset {
            return Err(Error::Protocol(format!(
                "cannot resume {path} at {offset}: remote file has only {size} bytes"
            )));
        }
        self.ftp
            .resume_transfer(offset as usize)
            .await
            .map_err(|e| match reply_code(&e) {
                Some(500 | 501 | 502 | 504) => Error::Unsupported("resuming transfers (REST)"),
                _ => denied(e),
            })?;
        match self.ftp.put_with_stream(path.as_str()).await {
            Ok(s) => Ok(s),
            Err(e) => {
                self.clear_rest(offset).await;
                Err(denied(e))
            }
        }
    }

    /// A pending `REST` would otherwise apply to the next transfer.
    async fn clear_rest(&mut self, offset: u64) {
        if offset > 0 {
            let _ = self.ftp.resume_transfer(0).await;
        }
    }
}

type TransferStreamT = suppaftp::tokio::TransferStream<suppaftp::tokio::AsyncRustlsStream>;

#[async_trait]
impl RemoteFs for FtpFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>> {
        self.state.lock().await.list(path).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<Entry> {
        self.state.lock().await.stat(path).await
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxRead> {
        let mut st = self.state.clone().lock_owned().await;
        let stream = st.start_read(path, offset).await?;
        Ok(Box::new(FtpReader::new(stream, st)))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxWrite> {
        let mut st = self.state.clone().lock_owned().await;
        let stream = st.start_write(path, offset).await?;
        Ok(Box::new(FtpWriter::new(stream, st)))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let mut st = self.state.lock().await;
        let Err(e) = st.ftp.mkdir(path.as_str()).await else {
            return Ok(());
        };
        match map_err(e, path, Plain::Denied) {
            // Servers rarely say why; find out.
            Error::PermissionDenied(_) | Error::NotFound(_) => {
                if st.stat(path).await.is_ok() {
                    return Err(Error::AlreadyExists(path.clone()));
                }
                match path.parent() {
                    Some(p) if matches!(st.stat(&p).await, Err(Error::NotFound(_))) => {
                        Err(Error::NotFound(path.clone()))
                    }
                    _ => Err(Error::PermissionDenied(path.clone())),
                }
            }
            e => Err(e),
        }
    }

    async fn remove(&self, path: &RemotePath) -> Result<()> {
        let mut st = self.state.lock().await;
        let entry = st.stat(path).await?;
        let res = if entry.is_dir() {
            st.ftp.rmdir(path.as_str()).await
        } else {
            st.ftp.rm(path.as_str()).await
        };
        res.map_err(|e| map_err(e, path, Plain::Denied))
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.state
            .lock()
            .await
            .ftp
            .rename(from.as_str(), to.as_str())
            .await
            .map_err(|e| nf(e, from))
    }
}
