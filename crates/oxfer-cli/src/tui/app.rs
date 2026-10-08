//! Pure TUI state: panes, popups and the transfer panel. No terminal I/O.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use oxfer_core::transfer::{Control, Event, JobId};
use oxfer_core::{Entry, EntryKind, RemotePath};

pub const LOCAL: usize = 0;
pub const REMOTE: usize = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Time,
}

impl SortKey {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Time,
            Self::Time => Self::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Time => "time",
        }
    }
}

pub struct Pane {
    pub label: String,
    pub cwd: RemotePath,
    entries: Vec<Entry>,
    pub cursor: usize,
    pub selected: HashSet<String>,
    pub filter: String,
    pub sort: SortKey,
    pub show_hidden: bool,
}

impl Pane {
    pub fn new(label: impl Into<String>, cwd: RemotePath) -> Self {
        Self {
            label: label.into(),
            cwd,
            entries: Vec::new(),
            cursor: 0,
            selected: HashSet::new(),
            filter: String::new(),
            sort: SortKey::Name,
            show_hidden: false,
        }
    }

    /// Entries as displayed: `..` first, then directories, then files, each
    /// group sorted, with hidden files and the filter applied.
    pub fn view(&self) -> Vec<Entry> {
        let filter = self.filter.to_lowercase();
        let mut v: Vec<Entry> = self
            .entries
            .iter()
            .filter(|e| self.show_hidden || !e.name().starts_with('.'))
            .filter(|e| filter.is_empty() || e.name().to_lowercase().contains(&filter))
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            let by_name = || a.name().to_lowercase().cmp(&b.name().to_lowercase());
            b.is_dir().cmp(&a.is_dir()).then_with(|| match self.sort {
                SortKey::Name => by_name(),
                SortKey::Size => b.size.cmp(&a.size).then_with(by_name),
                SortKey::Time => b.modified.cmp(&a.modified).then_with(by_name),
            })
        });
        if self.cwd.parent().is_some() {
            v.insert(
                0,
                Entry {
                    path: self.cwd.join(".."),
                    kind: EntryKind::Dir,
                    size: 0,
                    modified: None,
                    permissions: None,
                },
            );
        }
        v
    }

    pub fn current(&self) -> Option<Entry> {
        self.view().into_iter().nth(self.cursor)
    }

    pub fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.view().len().saturating_sub(1));
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.view().len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    /// Install a fresh listing for `cwd`; `focus` names the entry to select.
    pub fn set_listing(&mut self, cwd: RemotePath, entries: Vec<Entry>, focus: Option<&str>) {
        if cwd != self.cwd {
            self.selected.clear();
            self.filter.clear();
        }
        self.cwd = cwd;
        self.entries = entries
            .into_iter()
            .filter(|e| !matches!(e.name(), "." | ".."))
            .collect();
        let names: HashSet<_> = self.entries.iter().map(|e| e.name().to_string()).collect();
        self.selected.retain(|n| names.contains(n));
        let view = self.view();
        self.cursor = focus
            .and_then(|f| view.iter().position(|e| e.name() == f))
            .unwrap_or(0)
            .min(view.len().saturating_sub(1));
    }

    pub fn toggle_select(&mut self) {
        if let Some(e) = self.current().filter(|e| e.name() != "..") {
            let name = e.name().to_string();
            if !self.selected.remove(&name) {
                self.selected.insert(name);
            }
        }
    }

    /// Select everything visible, or clear if it is all selected already.
    pub fn select_all(&mut self) {
        let names: Vec<_> = self
            .view()
            .iter()
            .map(|e| e.name().to_string())
            .filter(|n| n != "..")
            .collect();
        if names.iter().all(|n| self.selected.contains(n)) {
            self.selected.clear();
        } else {
            self.selected.extend(names);
        }
    }

    /// The marked entries, or the one under the cursor if nothing is marked.
    pub fn targets(&self) -> Vec<Entry> {
        let view = self.view();
        if self.selected.is_empty() {
            return view
                .into_iter()
                .nth(self.cursor)
                .filter(|e| e.name() != "..")
                .into_iter()
                .collect();
        }
        view.into_iter()
            .filter(|e| self.selected.contains(e.name()))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputKind {
    Mkdir,
    Rename(RemotePath),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Help,
    Input { kind: InputKind, text: String },
    Confirm { paths: Vec<RemotePath> },
}

/// Side effects requested by the state machine; executed by the event loop.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    List {
        side: usize,
        path: RemotePath,
        focus: Option<String>,
    },
    Copy {
        from: usize,
        sources: Vec<RemotePath>,
        dest: RemotePath,
    },
    Mkdir {
        side: usize,
        path: RemotePath,
    },
    Rename {
        side: usize,
        from: RemotePath,
        to: RemotePath,
    },
    Delete {
        side: usize,
        paths: Vec<RemotePath>,
    },
}

/// Progress of the running (or last finished) transfer.
pub struct TransferState {
    pub control: Control,
    pub dest: usize,
    start: Instant,
    ended: Option<Duration>,
    pub files: usize,
    pub bytes: u64,
    progress: HashMap<JobId, u64>,
    finished_bytes: u64,
    pub files_done: usize,
    names: HashMap<JobId, String>,
    pub active: BTreeMap<JobId, String>,
    pub log: Vec<String>,
    pub summary: Option<String>,
}

impl TransferState {
    pub fn new(control: Control, dest: usize) -> Self {
        Self {
            control,
            dest,
            start: Instant::now(),
            ended: None,
            files: 0,
            bytes: 0,
            progress: HashMap::new(),
            finished_bytes: 0,
            files_done: 0,
            names: HashMap::new(),
            active: BTreeMap::new(),
            log: Vec::new(),
            summary: None,
        }
    }

    pub fn running(&self) -> bool {
        self.summary.is_none()
    }

    pub fn done_bytes(&self) -> u64 {
        self.finished_bytes + self.progress.values().sum::<u64>()
    }

    pub fn ratio(&self) -> f64 {
        match (self.bytes, self.files) {
            (0, 0) => 0.0,
            (0, f) => self.files_done as f64 / f as f64,
            (b, _) => (self.done_bytes() as f64 / b as f64).min(1.0),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(|| self.start.elapsed())
    }

    /// Average bytes per second since the start.
    pub fn speed(&self) -> f64 {
        self.done_bytes() as f64 / self.elapsed().as_secs_f64().max(0.001)
    }

    pub fn eta(&self) -> Option<Duration> {
        let speed = self.speed();
        (speed > 0.0 && self.running()).then(|| {
            Duration::from_secs_f64(self.bytes.saturating_sub(self.done_bytes()) as f64 / speed)
        })
    }

    pub fn on_event(&mut self, ev: Event) {
        let name = |s: &Self, id| s.names.get(&id).cloned().unwrap_or(format!("#{id}"));
        match ev {
            Event::Planned { files, bytes } => (self.files, self.bytes) = (files, bytes),
            Event::Started { id, src, .. } => {
                let n = src.file_name().unwrap_or("?").to_string();
                self.names.insert(id, n.clone());
                self.active.insert(id, n);
            }
            Event::Progress { id, done, .. } => {
                self.progress.insert(id, done);
            }
            Event::Retrying { id, attempt, error } => {
                let line = format!("retry {attempt} {}: {error}", name(self, id));
                self.log.push(line);
            }
            Event::Skipped { id, reason } => {
                let line = format!("skipped {}: {reason}", name(self, id));
                self.log.push(line);
                self.files_done += 1;
                self.active.remove(&id);
            }
            Event::Finished { id, bytes } => {
                self.progress.remove(&id);
                self.active.remove(&id);
                self.finished_bytes += bytes;
                self.files_done += 1;
            }
            Event::Failed { id, error } => {
                let line = format!("failed {}: {error}", name(self, id));
                self.log.push(line);
                self.progress.remove(&id);
                self.active.remove(&id);
                self.files_done += 1;
            }
            Event::Done {
                completed,
                skipped,
                failed,
                bytes,
                cancelled,
            } => {
                self.ended = Some(self.start.elapsed());
                self.active.clear();
                self.summary = Some(format!(
                    "{}{completed} done, {skipped} skipped, {failed} failed ({bytes} bytes)",
                    if cancelled { "cancelled: " } else { "" }
                ));
            }
        }
    }
}

pub struct App {
    pub panes: [Pane; 2],
    pub active: usize,
    pub popup: Option<Popup>,
    pub filtering: bool,
    pub status: String,
    pub status_err: bool,
    pub transfer: Option<TransferState>,
    pub quit: bool,
    /// Rows moved by PgUp/PgDn; set from the terminal height.
    pub page: usize,
}

impl App {
    pub fn new(local: Pane, remote: Pane) -> Self {
        Self {
            panes: [local, remote],
            active: LOCAL,
            popup: None,
            filtering: false,
            status: "Press ? for help".into(),
            status_err: false,
            transfer: None,
            quit: false,
            page: 10,
        }
    }

    pub fn pane(&self) -> &Pane {
        &self.panes[self.active]
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        (self.status, self.status_err) = (msg.into(), false);
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        (self.status, self.status_err) = (msg.into(), true);
    }

    /// Re-list `side` keeping the cursor on the same entry.
    pub fn refresh(&self, side: usize) -> Action {
        let p = &self.panes[side];
        Action::List {
            side,
            path: p.cwd.clone(),
            focus: p.current().map(|e| e.name().to_string()),
        }
    }

    pub fn start_transfer(&mut self, control: Control, dest: usize) {
        self.transfer = Some(TransferState::new(control, dest));
    }

    /// Feed a transfer event; returns a refresh of the destination when done.
    pub fn on_transfer_event(&mut self, ev: Event) -> Option<Action> {
        let t = self.transfer.as_mut()?;
        let done = matches!(ev, Event::Done { .. });
        t.on_event(ev);
        if done {
            let (dest, msg) = (t.dest, t.summary.clone().unwrap_or_default());
            self.info(msg);
            return Some(self.refresh(dest));
        }
        None
    }

    /// Whether a transfer is in flight (blocks starting another).
    pub fn transfer_running(&self) -> bool {
        self.transfer.as_ref().is_some_and(|t| t.running())
    }
}
