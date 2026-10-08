//! Key handling: a pure function of state and key, returning actions to run.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{Action, App, InputKind, Popup, LOCAL, REMOTE};

impl App {
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if key.kind == KeyEventKind::Release {
            return vec![];
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit_now();
            return vec![];
        }
        if self.popup.is_some() {
            return self.popup_key(key.code);
        }
        if self.filtering {
            return self.filter_key(key.code);
        }
        self.normal_key(key.code)
    }

    pub fn quit_now(&mut self) {
        if let Some(t) = &self.transfer {
            t.control.cancel();
        }
        self.quit = true;
    }

    fn filter_key(&mut self, code: KeyCode) -> Vec<Action> {
        match code {
            KeyCode::Enter => self.filtering = false,
            KeyCode::Esc => {
                self.filtering = false;
                self.pane_mut().filter.clear();
            }
            KeyCode::Backspace => {
                self.pane_mut().filter.pop();
            }
            KeyCode::Char(c) => self.pane_mut().filter.push(c),
            KeyCode::Up => self.pane_mut().move_by(-1),
            KeyCode::Down => self.pane_mut().move_by(1),
            _ => {}
        }
        self.pane_mut().clamp();
        vec![]
    }

    fn popup_key(&mut self, code: KeyCode) -> Vec<Action> {
        let side = self.active;
        match (self.popup.take(), code) {
            (Some(Popup::Help), _) => {}
            (Some(Popup::Confirm { paths }), KeyCode::Char('y' | 'Y') | KeyCode::Enter) => {
                self.pane_mut().selected.clear();
                return vec![Action::Delete { side, paths }];
            }
            (Some(Popup::Confirm { .. }), _) => {}
            (Some(Popup::Input { .. }), KeyCode::Esc) => {}
            (Some(Popup::Input { kind, text }), KeyCode::Enter) => {
                let name = text.trim();
                if name.is_empty() || name.contains('/') {
                    self.error("invalid name");
                    self.popup = Some(Popup::Input { kind, text });
                    return vec![];
                }
                let dir = self.pane().cwd.clone();
                return vec![match kind {
                    InputKind::Mkdir => Action::Mkdir {
                        side,
                        path: dir.join(name),
                    },
                    InputKind::Rename(from) => Action::Rename {
                        side,
                        from,
                        to: dir.join(name),
                    },
                }];
            }
            (Some(Popup::Input { kind, mut text }), c) => {
                match c {
                    KeyCode::Char(ch) => text.push(ch),
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    _ => {}
                }
                self.popup = Some(Popup::Input { kind, text });
            }
            (None, _) => {}
        }
        vec![]
    }

    fn normal_key(&mut self, code: KeyCode) -> Vec<Action> {
        let page = self.page.max(1) as isize;
        match code {
            KeyCode::Tab | KeyCode::BackTab => self.active = 1 - self.active,
            KeyCode::Up | KeyCode::Char('k') => self.pane_mut().move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.pane_mut().move_by(1),
            KeyCode::PageUp => self.pane_mut().move_by(-page),
            KeyCode::PageDown => self.pane_mut().move_by(page),
            KeyCode::Home | KeyCode::Char('g') => self.pane_mut().cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.pane_mut().move_by(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => return self.enter(),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => return self.go_up(),
            KeyCode::Char(' ') => {
                self.pane_mut().toggle_select();
                self.pane_mut().move_by(1);
            }
            KeyCode::Char('a') => self.pane_mut().select_all(),
            KeyCode::F(5) | KeyCode::Char('c') => return self.copy(),
            KeyCode::F(7) | KeyCode::Char('m') => {
                self.popup = Some(Popup::Input {
                    kind: InputKind::Mkdir,
                    text: String::new(),
                });
            }
            KeyCode::F(8) | KeyCode::Char('d') => {
                let paths: Vec<_> = self.pane().targets().into_iter().map(|e| e.path).collect();
                if paths.is_empty() {
                    self.error("nothing to delete");
                } else {
                    self.popup = Some(Popup::Confirm { paths });
                }
            }
            KeyCode::Char('r') => match self.pane().current().filter(|e| e.name() != "..") {
                Some(e) => {
                    self.popup = Some(Popup::Input {
                        text: e.name().to_string(),
                        kind: InputKind::Rename(e.path),
                    });
                }
                None => self.error("nothing to rename"),
            },
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('s') => {
                let p = self.pane_mut();
                p.sort = p.sort.next();
                p.cursor = 0;
                let label = p.sort.label();
                self.info(format!("sort by {label}"));
            }
            KeyCode::Char('.') => {
                let p = self.pane_mut();
                p.show_hidden = !p.show_hidden;
                p.clamp();
            }
            KeyCode::Char('p') => {
                if let Some(t) = self.transfer.as_ref().filter(|t| t.running()) {
                    if t.control.is_paused() {
                        t.control.resume();
                    } else {
                        t.control.pause();
                    }
                }
            }
            KeyCode::Esc => {
                if self.transfer_running() {
                    if let Some(t) = &self.transfer {
                        t.control.cancel();
                    }
                    self.info("cancelling transfer...");
                } else if self.transfer.is_some() {
                    self.transfer = None;
                } else {
                    self.pane_mut().filter.clear();
                    self.pane_mut().clamp();
                }
            }
            KeyCode::F(10) | KeyCode::Char('q') => self.quit_now(),
            KeyCode::Char('?') | KeyCode::F(1) => self.popup = Some(Popup::Help),
            _ => {}
        }
        vec![]
    }

    fn enter(&mut self) -> Vec<Action> {
        let Some(e) = self.pane().current() else {
            return vec![];
        };
        if e.name() == ".." {
            return self.go_up();
        }
        if !e.is_dir() && e.kind != oxfer_core::EntryKind::Symlink {
            return vec![];
        }
        vec![Action::List {
            side: self.active,
            path: e.path,
            focus: None,
        }]
    }

    fn go_up(&mut self) -> Vec<Action> {
        let cwd = &self.pane().cwd;
        match cwd.parent() {
            Some(path) => vec![Action::List {
                side: self.active,
                path,
                focus: cwd.file_name().map(String::from),
            }],
            None => vec![],
        }
    }

    fn copy(&mut self) -> Vec<Action> {
        if self.transfer_running() {
            self.error("a transfer is already running");
            return vec![];
        }
        let sources: Vec<_> = self.pane().targets().into_iter().map(|e| e.path).collect();
        if sources.is_empty() {
            self.error("nothing to copy");
            return vec![];
        }
        let from = self.active;
        let to = if from == LOCAL { REMOTE } else { LOCAL };
        self.pane_mut().selected.clear();
        vec![Action::Copy {
            from,
            sources,
            dest: self.panes[to].cwd.clone(),
        }]
    }
}
