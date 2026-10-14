//! [`SessionManager`]: open sessions, file operations and running transfers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use oxfer_core::transfer::{Control, Job, Plan, Transfer};
use oxfer_core::{remove_all, LocalFs, RemoteFs, RemotePath, ShellControl};
use tokio_util::sync::CancellationToken;

use super::connect::{self, ConnectEnv, ConnectRequest};
use super::dto::{EntryDto, ProtocolDto, SessionInfo};
use super::editor::{self, FilePreview, Opener};
use super::error::{ServiceError, ServiceResult};
use super::profiles::ProfileStore;
use super::prompt::Prompter;
use super::terminal::{TerminalEvent, TerminalSink};
use super::transfers::{
    conflict_resolver, core_options, forward_events, EventSink, OverwriteDto, SkippedDto,
    TransferEvent, TransferOptionsDto, TransferRequest, TransferState,
};
use super::vnc::{VncHub, VncInfo, VncSink};

/// Id of the built-in local file system session.
pub const LOCAL_SESSION: &str = "local";

struct Session {
    fs: Arc<dyn RemoteFs>,
    info: SessionInfo,
    /// Server keys this session answers for (see `connect::server_key`).
    keys: Vec<String>,
}

/// Marks a server as being connected to until dropped.
struct Connecting<'a> {
    set: &'a Mutex<std::collections::HashSet<String>>,
    key: String,
}

impl Drop for Connecting<'_> {
    fn drop(&mut self) {
        lock(self.set).remove(&self.key);
    }
}

struct Terminal {
    session: String,
    control: Arc<dyn ShellControl>,
}

struct TransferEntry {
    request: TransferRequest,
    control: Control,
    cancel: CancellationToken,
    state: Arc<Mutex<TransferState>>,
}

/// Owns all connections and transfers. Has no UI dependencies: questions go
/// through a [`Prompter`], progress through an [`EventSink`].
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Session>>,
    next_session: AtomicU64,
    transfers: Mutex<HashMap<u64, TransferEntry>>,
    next_transfer: AtomicU64,
    sink: Arc<dyn EventSink>,
    profiles: ProfileStore,
    env: ConnectEnv,
    opener: RwLock<Opener>,
    /// Servers a connection is currently being opened to.
    connecting: Mutex<std::collections::HashSet<String>>,
    /// Remote files open in an editor: (session, path) -> local copy.
    edits: Mutex<HashMap<(String, String), PathBuf>>,
    next_edit: AtomicU64,
    terminals: Mutex<HashMap<u64, Terminal>>,
    next_terminal: AtomicU64,
    vnc: VncHub,
}

fn local_home() -> String {
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    if cfg!(windows) {
        home.replace('\\', "/")
    } else {
        home
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_path(path: &str) -> ServiceResult<RemotePath> {
    if path.is_empty() || path.contains('\0') {
        return Err(ServiceError::invalid(format!("invalid path {path:?}")));
    }
    Ok(RemotePath::new(path))
}

impl SessionManager {
    pub fn new(profiles: ProfileStore, env: ConnectEnv, sink: Arc<dyn EventSink>) -> Arc<Self> {
        Self::with_local_home(profiles, env, sink, local_home())
    }

    /// Like [`new`](Self::new) but with an explicit start directory for the
    /// local session.
    pub fn with_local_home(
        profiles: ProfileStore,
        env: ConnectEnv,
        sink: Arc<dyn EventSink>,
        local_home: String,
    ) -> Arc<Self> {
        let mut sessions = HashMap::new();
        sessions.insert(
            LOCAL_SESSION.to_string(),
            Session {
                fs: Arc::new(LocalFs),
                keys: Vec::new(),
                info: SessionInfo {
                    id: LOCAL_SESSION.into(),
                    label: "Local".into(),
                    protocol: ProtocolDto::Local,
                    home: local_home,
                },
            },
        );
        Arc::new(Self {
            sessions: RwLock::new(sessions),
            next_session: AtomicU64::new(1),
            transfers: Mutex::new(HashMap::new()),
            next_transfer: AtomicU64::new(1),
            sink,
            profiles,
            env,
            opener: RwLock::new(editor::system_opener()),
            connecting: Mutex::new(Default::default()),
            edits: Mutex::new(HashMap::new()),
            next_edit: AtomicU64::new(1),
            terminals: Mutex::new(HashMap::new()),
            next_terminal: AtomicU64::new(1),
            vnc: VncHub::default(),
        })
    }

    /// Replace how files are opened in an editor (tests, embedders).
    pub fn set_opener(&self, opener: Opener) {
        *self.opener.write().unwrap_or_else(|e| e.into_inner()) = opener;
    }

    async fn open_local(&self, path: &Path) -> ServiceResult<()> {
        let opener = self
            .opener
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || opener(&path))
            .await
            .map_err(|e| ServiceError::new("internal", e.to_string()))?
            .map_err(|e| ServiceError::new("io", format!("cannot open an editor: {e}")))
    }

    /// The first [`PREVIEW_LIMIT`](editor::PREVIEW_LIMIT) bytes of a file as text.
    pub async fn read_preview(&self, session: &str, path: &str) -> ServiceResult<FilePreview> {
        let fs = self.fs(session)?;
        editor::read_preview(&*fs, &check_path(path)?).await
    }

    /// Open a file in the text editor. Remote files are edited as a local
    /// copy that is uploaded back whenever it is saved, until the session closes.
    pub async fn open_in_editor(
        self: &Arc<Self>,
        session: &str,
        path: &str,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<String> {
        let remote = check_path(path)?;
        let fs = self.fs(session)?;
        if session == LOCAL_SESSION {
            self.open_local(Path::new(remote.as_str())).await?;
            return Ok(remote.to_string());
        }
        let key = (session.to_string(), remote.to_string());
        let existing = lock(&self.edits).get(&key).cloned();
        if let Some(local) = existing {
            self.open_local(&local).await?;
            return Ok(local.to_string_lossy().into_owned());
        }
        let entry = fs.stat(&remote).await?;
        if entry.kind != oxfer_core::EntryKind::File {
            return Err(ServiceError::invalid(format!("{remote} is not a file")));
        }
        if entry.size > editor::EDIT_LIMIT {
            return Err(ServiceError::invalid(format!(
                "{remote} is too large to edit ({} bytes)",
                entry.size
            )));
        }
        let parent = remote
            .parent()
            .ok_or_else(|| ServiceError::invalid(format!("{remote} has no parent directory")))?;

        let dir = std::env::temp_dir().join(format!(
            "oxfer-edit-{}-{}",
            std::process::id(),
            self.next_edit.fetch_add(1, Ordering::Relaxed)
        ));
        create_private_dir(&dir)?;
        let local = dir.join(editor::local_name(&remote));
        if let Err(e) = editor::download(&*fs, &remote, &local).await {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        lock(&self.edits).insert(key.clone(), local.clone());
        tokio::spawn(watch_edit(
            Arc::downgrade(self),
            key,
            local.clone(),
            parent,
            prompter,
        ));
        self.open_local(&local).await?;
        Ok(local.to_string_lossy().into_owned())
    }

    pub fn profiles(&self) -> &ProfileStore {
        &self.profiles
    }

    /// Register an already connected file system (used by tests and embedders).
    pub fn add_session(
        &self,
        fs: Arc<dyn RemoteFs>,
        label: &str,
        protocol: ProtocolDto,
        home: &str,
    ) -> SessionInfo {
        self.add_session_with_keys(fs, label, protocol, home, Vec::new())
    }

    fn add_session_with_keys(
        &self,
        fs: Arc<dyn RemoteFs>,
        label: &str,
        protocol: ProtocolDto,
        home: &str,
        keys: Vec<String>,
    ) -> SessionInfo {
        let id = format!("s{}", self.next_session.fetch_add(1, Ordering::Relaxed));
        let info = SessionInfo {
            id: id.clone(),
            label: label.into(),
            protocol,
            home: home.into(),
        };
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Session {
                    fs,
                    info: info.clone(),
                    keys,
                },
            );
        info
    }

    pub fn sessions(&self) -> Vec<SessionInfo> {
        let mut v: Vec<_> = self
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.info.clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn fs(&self, session: &str) -> ServiceResult<Arc<dyn RemoteFs>> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .map(|s| s.fs.clone())
            .ok_or_else(|| ServiceError::unknown_session(session))
    }

    /// The open session for `key`, remembering `also` as another key for it.
    fn session_for(&self, key: &str, also: Option<&String>) -> Option<SessionInfo> {
        let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        let s = sessions
            .values_mut()
            .find(|s| s.keys.iter().any(|k| k == key))?;
        if let Some(k) = also.filter(|k| !s.keys.contains(k)) {
            s.keys.push(k.clone());
        }
        Some(s.info.clone())
    }

    /// Connect, or return the session already open to the same server and
    /// account: there is at most one session per server.
    pub async fn connect(
        &self,
        req: ConnectRequest,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<SessionInfo> {
        let requested = connect::request_key(&self.profiles, &req);
        let _connecting = match &requested {
            Some(k) => {
                if let Some(existing) = self.session_for(k, None) {
                    return Ok(existing);
                }
                if !lock(&self.connecting).insert(k.clone()) {
                    return Err(ServiceError::new(
                        "busy",
                        "a connection to this server is already being opened",
                    ));
                }
                Some(Connecting {
                    set: &self.connecting,
                    key: k.clone(),
                })
            }
            None => None,
        };
        let c = connect::connect(&self.env, &self.profiles, req, prompter).await?;
        // Reached under another name (e.g. an ssh_config alias): keep the
        // first session and close the new connection.
        if let Some(existing) = self.session_for(&c.key, requested.as_ref()) {
            return Ok(existing);
        }
        let home = connect::start_dir(&*c.fs, c.start_dir).await;
        let keys = std::iter::once(c.key).chain(requested).collect();
        Ok(self.add_session_with_keys(c.fs, &c.label, c.protocol, &home, keys))
    }

    /// Close a session. Running transfers keep their own reference and finish.
    pub fn disconnect(&self, session: &str) -> ServiceResult<()> {
        if session == LOCAL_SESSION {
            return Err(ServiceError::invalid("the local session cannot be closed"));
        }
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session)
            .ok_or_else(|| ServiceError::unknown_session(session))?;
        let closing: Vec<u64> = lock(&self.terminals)
            .iter()
            .filter(|(_, t)| t.session == session)
            .map(|(id, _)| *id)
            .collect();
        for id in closing {
            let _ = self.close_terminal(id);
        }
        Ok(())
    }

    /// Open an interactive shell on `session`. Output and the end of the
    /// shell are reported through `sink`; returns the terminal id.
    pub async fn open_terminal(
        &self,
        session: &str,
        cols: u32,
        rows: u32,
        sink: Arc<dyn TerminalSink>,
    ) -> ServiceResult<u64> {
        if session == LOCAL_SESSION {
            return Err(ServiceError::invalid(
                "a terminal needs a connection to a server",
            ));
        }
        let fs = self.fs(session)?;
        let oxfer_core::Shell {
            mut output,
            control,
        } = fs.open_shell(cols.max(1), rows.max(1)).await?;
        let id = self.next_terminal.fetch_add(1, Ordering::Relaxed);
        lock(&self.terminals).insert(
            id,
            Terminal {
                session: session.to_string(),
                control,
            },
        );
        tokio::spawn(async move {
            while let Some(data) = output.recv().await {
                sink.emit(TerminalEvent::Output {
                    terminal_id: id,
                    data,
                });
            }
            sink.emit(TerminalEvent::Closed { terminal_id: id });
        });
        Ok(id)
    }

    fn terminal(&self, id: u64) -> ServiceResult<Arc<dyn ShellControl>> {
        lock(&self.terminals)
            .get(&id)
            .map(|t| t.control.clone())
            .ok_or_else(|| ServiceError::new("unknownTerminal", format!("no terminal #{id}")))
    }

    pub async fn terminal_write(&self, id: u64, data: &[u8]) -> ServiceResult<()> {
        Ok(self.terminal(id)?.write(data).await?)
    }

    pub async fn terminal_resize(&self, id: u64, cols: u32, rows: u32) -> ServiceResult<()> {
        Ok(self.terminal(id)?.resize(cols.max(1), rows.max(1)).await?)
    }

    /// Close a terminal; the shell ends and `Closed` is emitted.
    pub fn close_terminal(&self, id: u64) -> ServiceResult<()> {
        let t = lock(&self.terminals)
            .remove(&id)
            .ok_or_else(|| ServiceError::new("unknownTerminal", format!("no terminal #{id}")))?;
        tokio::spawn(async move {
            let _ = t.control.close().await;
        });
        Ok(())
    }

    /// Open a remote screen (a VNC profile or quick connect). Screen
    /// messages go to `sink`; returns the screen's id.
    pub async fn vnc_open(
        &self,
        request: ConnectRequest,
        prompter: Arc<dyn Prompter>,
        sink: Arc<dyn VncSink>,
    ) -> ServiceResult<VncInfo> {
        self.vnc.open(&self.profiles, request, prompter, sink).await
    }

    pub fn vnc_key(&self, id: u64, keysym: u32, down: bool) -> ServiceResult<()> {
        self.vnc.key(id, keysym, down)
    }

    pub fn vnc_pointer(&self, id: u64, x: u16, y: u16, buttons: u8) -> ServiceResult<()> {
        self.vnc.pointer(id, x, y, buttons)
    }

    pub fn vnc_clipboard(&self, id: u64, text: String) -> ServiceResult<()> {
        self.vnc.clipboard(id, text)
    }

    pub fn vnc_close(&self, id: u64) -> ServiceResult<()> {
        self.vnc.close(id)
    }

    pub async fn list_dir(&self, session: &str, path: &str) -> ServiceResult<Vec<EntryDto>> {
        let fs = self.fs(session)?;
        let entries = fs.list(&check_path(path)?).await?;
        Ok(entries
            .iter()
            .filter(|e| !matches!(e.name(), "." | ".."))
            .map(EntryDto::from)
            .collect())
    }

    pub async fn stat(&self, session: &str, path: &str) -> ServiceResult<EntryDto> {
        let fs = self.fs(session)?;
        Ok((&fs.stat(&check_path(path)?).await?).into())
    }

    /// Canonical form of `path` (e.g. to resolve a typed-in path).
    pub async fn realpath(&self, session: &str, path: &str) -> ServiceResult<String> {
        let fs = self.fs(session)?;
        Ok(fs.realpath(&check_path(path)?).await?.to_string())
    }

    pub async fn mkdir(&self, session: &str, path: &str) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        Ok(fs.mkdir(&check_path(path)?).await?)
    }

    pub async fn rename(&self, session: &str, from: &str, to: &str) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        let (from, to) = (check_path(from)?, check_path(to)?);
        // Never silently replace an existing entry.
        match fs.lstat(&to).await {
            Ok(_) => return Err(oxfer_core::Error::AlreadyExists(to).into()),
            Err(oxfer_core::Error::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(fs.rename(&from, &to).await?)
    }

    /// Delete files and directories (recursively).
    pub async fn delete(&self, session: &str, paths: &[String]) -> ServiceResult<()> {
        let fs = self.fs(session)?;
        for p in paths {
            let p = check_path(p)?;
            if p.parent().is_none() && p.is_absolute() {
                return Err(ServiceError::invalid(
                    "refusing to delete the root directory",
                ));
            }
            remove_all(&*fs, &p).await?;
        }
        Ok(())
    }

    /// Start copying `sources` into `dest_dir`. Returns the transfer id; all
    /// progress is reported through the [`EventSink`].
    pub async fn start_transfer(
        &self,
        req: TransferRequest,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<u64> {
        if req.sources.is_empty() {
            return Err(ServiceError::invalid("nothing to transfer"));
        }
        let dest = check_path(&req.dest_dir)?;
        let sources = req
            .sources
            .iter()
            .map(|s| check_path(s))
            .collect::<ServiceResult<Vec<_>>>()?;
        if req.src_session == req.dst_session {
            let d = dest.normalize();
            for s in &sources {
                let s = s.normalize();
                if s.parent().as_ref() == Some(&d) || d == s {
                    return Err(ServiceError::invalid(format!(
                        "{s} is already in {d}; source and destination are the same"
                    )));
                }
                if d.as_str()
                    .starts_with(&format!("{}/", s.as_str().trim_end_matches('/')))
                {
                    return Err(ServiceError::invalid(format!(
                        "cannot copy {s} into itself"
                    )));
                }
            }
        }
        self.spawn_transfer(req, None, None, prompter)
    }

    fn spawn_transfer(
        &self,
        req: TransferRequest,
        plan: Option<Plan>,
        retry_of: Option<u64>,
        prompter: Arc<dyn Prompter>,
    ) -> ServiceResult<u64> {
        let src = self.fs(&req.src_session)?;
        let dst = self.fs(&req.dst_session)?;
        let id = self.next_transfer.fetch_add(1, Ordering::Relaxed);
        let control = Control::new();
        let cancel = CancellationToken::new();
        let state = Arc::new(Mutex::new(TransferState::default()));
        lock(&self.transfers).insert(
            id,
            TransferEntry {
                request: req.clone(),
                control: control.clone(),
                cancel: cancel.clone(),
                state: state.clone(),
            },
        );
        self.sink.emit(TransferEvent::Queued {
            transfer_id: id,
            src_session: req.src_session.clone(),
            dst_session: req.dst_session.clone(),
            sources: req.sources.clone(),
            dest_dir: req.dest_dir.clone(),
            retry_of,
        });

        let opts = core_options(
            &req.options,
            conflict_resolver(id, prompter, cancel.clone()),
        );
        let transfer = Transfer::new(src, dst, opts);
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let plan = match plan {
                Some(p) => Ok(p),
                None => {
                    let sources: Vec<RemotePath> =
                        req.sources.iter().map(RemotePath::new).collect();
                    let dest = RemotePath::new(req.dest_dir.clone());
                    tokio::select! {
                        p = transfer.plan(&sources, &dest) => p,
                        _ = cancel.cancelled() => Err(oxfer_core::Error::Cancelled),
                    }
                }
            };
            let plan = match plan {
                Ok(p) => p,
                Err(e) => {
                    let cancelled = matches!(e, oxfer_core::Error::Cancelled);
                    {
                        let mut st = lock(&state);
                        st.done = true;
                        st.plan_failed = true;
                    }
                    if !cancelled {
                        sink.emit(TransferEvent::Failed {
                            transfer_id: id,
                            job_id: None,
                            path: req.dest_dir.clone(),
                            error: e.to_string(),
                        });
                    }
                    sink.emit(TransferEvent::Done {
                        transfer_id: id,
                        completed: 0,
                        skipped: 0,
                        failed: usize::from(!cancelled),
                        bytes: 0,
                        cancelled,
                    });
                    return;
                }
            };
            {
                let mut st = lock(&state);
                st.dirs = plan.dirs.clone();
                st.jobs = plan.jobs.clone();
                st.plan_skipped = plan
                    .skipped
                    .iter()
                    .map(|(p, r)| SkippedDto {
                        path: p.to_string(),
                        reason: r.clone(),
                    })
                    .collect();
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let fwd = tokio::spawn(forward_events(id, rx, sink, state));
            transfer.run(plan, Some(tx), control).await;
            let _ = fwd.await;
        });
        Ok(id)
    }

    fn with_transfer<T>(
        &self,
        id: u64,
        f: impl FnOnce(&TransferEntry) -> ServiceResult<T>,
    ) -> ServiceResult<T> {
        let map = lock(&self.transfers);
        let entry = map
            .get(&id)
            .ok_or_else(|| ServiceError::unknown_transfer(id))?;
        f(entry)
    }

    pub fn pause_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.control.pause();
            Ok(())
        })?;
        self.sink.emit(TransferEvent::Paused { transfer_id: id });
        Ok(())
    }

    pub fn resume_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.control.resume();
            Ok(())
        })?;
        self.sink.emit(TransferEvent::Resumed { transfer_id: id });
        Ok(())
    }

    pub fn cancel_transfer(&self, id: u64) -> ServiceResult<()> {
        self.with_transfer(id, |t| {
            t.cancel.cancel();
            t.control.cancel();
            Ok(())
        })
    }

    pub fn is_transfer_done(&self, id: u64) -> ServiceResult<bool> {
        self.with_transfer(id, |t| Ok(lock(&t.state).done))
    }

    /// Run the files of a finished transfer that did not complete (failed or
    /// cancelled) again, as a new transfer.
    pub async fn retry_transfer(&self, id: u64, prompter: Arc<dyn Prompter>) -> ServiceResult<u64> {
        let (req, plan) = self.with_transfer(id, |t| {
            let st = lock(&t.state);
            if !st.done {
                return Err(ServiceError::invalid("the transfer is still running"));
            }
            if st.plan_failed {
                return Ok((t.request.clone(), None));
            }
            let jobs: Vec<Job> = st
                .jobs
                .iter()
                .filter(|j| !st.settled.contains(&j.id))
                .enumerate()
                .map(|(i, j)| Job {
                    id: i,
                    src: j.src.clone(),
                    dst: j.dst.clone(),
                })
                .collect();
            if jobs.is_empty() {
                return Err(ServiceError::invalid("nothing to retry"));
            }
            let plan = Plan {
                dirs: st.dirs.clone(),
                jobs,
                skipped: vec![],
            };
            Ok((t.request.clone(), Some(plan)))
        })?;
        self.spawn_transfer(req, plan, Some(id), prompter)
    }

    /// Drop finished transfers from the registry.
    pub fn clear_finished(&self) {
        lock(&self.transfers).retain(|_, t| !lock(&t.state).done);
    }
}

/// A directory only the current user can read (it holds copies of remote files).
fn create_private_dir(dir: &Path) -> ServiceResult<()> {
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    Ok(())
}

/// Upload the edited copy `local` to `remote_dir` of `key.0` after every
/// save; stop (and delete the copy) when the session or the copy is gone.
async fn watch_edit(
    mgr: Weak<SessionManager>,
    key: (String, String),
    local: PathBuf,
    remote_dir: RemotePath,
    prompter: Arc<dyn Prompter>,
) {
    let (session, _) = &key;
    let mut uploaded = editor::stamp(&local);
    let mut pending = uploaded;
    loop {
        tokio::time::sleep(editor::WATCH_INTERVAL).await;
        let Some(m) = mgr.upgrade() else { break };
        if m.fs(session).is_err() {
            break;
        }
        let Some(now) = editor::stamp(&local) else {
            break;
        };
        // Upload once the file has stopped changing for one interval.
        if Some(now) == uploaded || Some(now) != pending {
            pending = Some(now);
            continue;
        }
        uploaded = Some(now);
        let req = TransferRequest {
            src_session: LOCAL_SESSION.into(),
            dst_session: session.clone(),
            sources: vec![RemotePath::from_local(&local.to_string_lossy()).to_string()],
            dest_dir: remote_dir.to_string(),
            options: TransferOptionsDto {
                overwrite: OverwriteDto::Overwrite,
                resume: false,
                verify: false,
                concurrency: Some(1),
            },
        };
        if let Err(e) = m.spawn_transfer(req, None, None, prompter.clone()) {
            tracing::warn!("cannot upload edited {}: {}", key.1, e.message);
        }
    }
    if let Some(m) = mgr.upgrade() {
        lock(&m.edits).remove(&key);
    }
    if let Some(dir) = local.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}
