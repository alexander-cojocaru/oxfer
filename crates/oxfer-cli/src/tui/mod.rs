//! Dual-pane terminal file manager: `oxfer tui user@host[:path]`.
//!
//! `app` holds the pure state, `input` maps keys to state changes and
//! [`Action`]s, `ui` renders; this module wires them to the terminal and runs
//! the file system work on tokio tasks so the UI loop never blocks.

mod app;
mod input;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use oxfer_core::transfer::{Control, Event, Transfer, TransferOptions};
use oxfer_core::{remove_all, Entry, LocalFs, RemoteFs, RemotePath};
use ratatui::crossterm::event::{self, Event as TermEvent};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use app::{Action, App, Pane};

/// Messages delivered to the UI loop.
enum Msg {
    Term(TermEvent),
    Listed {
        side: usize,
        path: RemotePath,
        focus: Option<String>,
        result: Result<Vec<Entry>, String>,
    },
    Transfer(Event),
    /// Planning failed before the transfer started.
    Aborted(String),
    /// A mkdir/rename/delete finished; refresh `side`.
    Op {
        side: usize,
        result: Result<String, String>,
    },
}

type Tx = UnboundedSender<Msg>;

fn spawn_terminal_reader(tx: Tx) {
    std::thread::spawn(move || {
        while !tx.is_closed() {
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                let sent = event::read().is_ok_and(|ev| tx.send(Msg::Term(ev)).is_ok());
                if !sent {
                    break;
                }
            }
        }
    });
}

struct Runner {
    fs: [Arc<dyn RemoteFs>; 2],
    tx: Tx,
}

impl Runner {
    fn exec(&self, app: &mut App, action: Action) {
        let tx = self.tx.clone();
        match action {
            Action::List { side, path, focus } => {
                let fs = self.fs[side].clone();
                tokio::spawn(async move {
                    let result = fs.list(&path).await.map_err(|e| e.to_string());
                    let _ = tx.send(Msg::Listed {
                        side,
                        path,
                        focus,
                        result,
                    });
                });
            }
            Action::Copy {
                from,
                sources,
                dest,
            } => {
                let to = 1 - from;
                let control = Control::new();
                app.start_transfer(control.clone(), to);
                app.info("transfer started");
                let transfer = Transfer::new(
                    self.fs[from].clone(),
                    self.fs[to].clone(),
                    TransferOptions {
                        recursive: true,
                        ..Default::default()
                    },
                );
                tokio::spawn(async move {
                    let plan = match transfer.plan(&sources, &dest).await {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx.send(Msg::Aborted(e.to_string()));
                            return;
                        }
                    };
                    let (etx, mut erx) = unbounded_channel();
                    let fwd = tokio::spawn(async move {
                        while let Some(ev) = erx.recv().await {
                            if tx.send(Msg::Transfer(ev)).is_err() {
                                break;
                            }
                        }
                    });
                    transfer.run(plan, Some(etx), control).await;
                    let _ = fwd.await;
                });
            }
            Action::Mkdir { side, path } => {
                let fs = self.fs[side].clone();
                tokio::spawn(async move {
                    let result = fs.mkdir(&path).await;
                    op_done(&tx, side, result, format!("created {path}"));
                });
            }
            Action::Rename { side, from, to } => {
                let fs = self.fs[side].clone();
                tokio::spawn(async move {
                    let result = fs.rename(&from, &to).await;
                    op_done(&tx, side, result, format!("renamed to {to}"));
                });
            }
            Action::Delete { side, paths } => {
                let fs = self.fs[side].clone();
                tokio::spawn(async move {
                    let mut result = Ok(());
                    for p in &paths {
                        if let Err(e) = remove_all(&*fs, p).await {
                            result = Err(e);
                            break;
                        }
                    }
                    op_done(
                        &tx,
                        side,
                        result,
                        format!("deleted {} item(s)", paths.len()),
                    );
                });
            }
        }
    }
}

fn op_done(tx: &Tx, side: usize, result: oxfer_core::Result<()>, ok: String) {
    let result = result.map(|()| ok).map_err(|e| e.to_string());
    let _ = tx.send(Msg::Op { side, result });
}

/// Apply a message to the state; returns follow-up actions.
fn handle(app: &mut App, msg: Msg) -> Vec<Action> {
    match msg {
        Msg::Term(TermEvent::Key(k)) => return app.on_key(k),
        Msg::Term(_) => {}
        Msg::Listed {
            side,
            path,
            focus,
            result,
        } => match result {
            Ok(entries) => app.panes[side].set_listing(path, entries, focus.as_deref()),
            Err(e) => app.error(format!("{}: {e}", path)),
        },
        Msg::Transfer(ev) => return app.on_transfer_event(ev).into_iter().collect(),
        Msg::Aborted(e) => {
            app.transfer = None;
            app.error(e);
        }
        Msg::Op { side, result } => {
            match result {
                Ok(m) => app.info(m),
                Err(e) => app.error(e),
            }
            return vec![app.refresh(side)];
        }
    }
    vec![]
}

pub async fn run(fs: Arc<dyn RemoteFs>, label: String, start_dir: &str) -> Result<()> {
    let remote_cwd = fs.realpath(&RemotePath::new(start_dir)).await?;
    let local_cwd = RemotePath::new(std::env::current_dir()?.to_string_lossy().into_owned());
    let mut app = App::new(
        Pane::new("local", local_cwd.clone()),
        Pane::new(label, remote_cwd.clone()),
    );
    let (tx, mut rx) = unbounded_channel();
    let runner = Runner {
        fs: [Arc::new(LocalFs), fs],
        tx: tx.clone(),
    };
    for (side, path) in [(0, local_cwd), (1, remote_cwd)] {
        runner.exec(
            &mut app,
            Action::List {
                side,
                path,
                focus: None,
            },
        );
    }
    spawn_terminal_reader(tx);

    let mut term = ratatui::try_init()?; // also installs the restoring panic hook
    let result = event_loop(&mut term, &mut app, &runner, &mut rx).await;
    ratatui::restore();
    result
}

async fn event_loop(
    term: &mut ratatui::DefaultTerminal,
    app: &mut App,
    runner: &Runner,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Msg>,
) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    while !app.quit {
        app.page = (term.size()?.height as usize).saturating_sub(12).max(1);
        term.draw(|f| ui::draw(f, app))?;
        let actions = tokio::select! {
            Some(msg) = rx.recv() => handle(app, msg),
            _ = tick.tick() => vec![],
        };
        for a in actions {
            runner.exec(app, a);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
