use std::time::{Duration, UNIX_EPOCH};

use oxfer_core::transfer::{Control, Event};
use oxfer_core::{Entry, EntryKind, RemotePath};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use super::app::*;
use super::ui;

fn ent(dir: &str, name: &str, kind: EntryKind, size: u64, mtime: u64) -> Entry {
    Entry {
        path: RemotePath::new(dir).join(name),
        kind,
        size,
        modified: Some(UNIX_EPOCH + Duration::from_secs(mtime)),
        permissions: None,
    }
}

fn listing(dir: &str) -> Vec<Entry> {
    use EntryKind::*;
    vec![
        ent(dir, "zeta.txt", File, 10, 300),
        ent(dir, "Alpha.log", File, 5000, 100),
        ent(dir, "docs", Dir, 0, 200),
        ent(dir, ".hidden", File, 1, 50),
        ent(dir, "bin", Dir, 0, 400),
    ]
}

fn app() -> App {
    let mut a = App::new(
        Pane::new("local", RemotePath::new("/home/me")),
        Pane::new("user@host", RemotePath::new("/srv")),
    );
    a.panes[0].set_listing(RemotePath::new("/home/me"), listing("/home/me"), None);
    a.panes[1].set_listing(RemotePath::new("/srv"), listing("/srv"), None);
    a
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn press(a: &mut App, keys: &str) -> Vec<Action> {
    keys.chars()
        .flat_map(|c| a.on_key(key(KeyCode::Char(c))))
        .collect()
}

fn names(p: &Pane) -> Vec<String> {
    p.view().iter().map(|e| e.name().to_string()).collect()
}

#[test]
fn view_is_up_dirs_first_sorted_without_hidden() {
    let a = app();
    assert_eq!(
        names(&a.panes[0]),
        ["..", "bin", "docs", "Alpha.log", "zeta.txt"]
    );
}

#[test]
fn sort_cycles_and_hidden_toggles() {
    let mut a = app();
    press(&mut a, "s");
    assert_eq!(
        names(a.pane()),
        ["..", "bin", "docs", "Alpha.log", "zeta.txt"]
    ); // size desc
    press(&mut a, "s");
    assert_eq!(a.pane().sort, SortKey::Time);
    assert_eq!(
        names(a.pane()),
        ["..", "bin", "docs", "zeta.txt", "Alpha.log"]
    );
    press(&mut a, "s.");
    assert_eq!(a.pane().sort, SortKey::Name);
    assert!(names(a.pane()).contains(&".hidden".to_string()));
}

#[test]
fn navigation_moves_and_clamps() {
    let mut a = app();
    press(&mut a, "jj");
    assert_eq!(a.pane().cursor, 2);
    press(&mut a, "k");
    assert_eq!(a.pane().cursor, 1);
    a.on_key(key(KeyCode::End));
    assert_eq!(a.pane().cursor, 4);
    press(&mut a, "j");
    assert_eq!(a.pane().cursor, 4);
    a.page = 3;
    a.on_key(key(KeyCode::PageUp));
    assert_eq!(a.pane().cursor, 1);
    a.on_key(key(KeyCode::PageUp));
    assert_eq!(a.pane().cursor, 0);
    a.on_key(key(KeyCode::Tab));
    assert_eq!(a.active, REMOTE);
    assert_eq!(a.pane().cursor, 0);
}

#[test]
fn enter_and_go_up_emit_list_actions() {
    let mut a = app();
    press(&mut a, "j");
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        [Action::List {
            side: LOCAL,
            path: RemotePath::new("/home/me/bin"),
            focus: None
        }]
    );
    assert_eq!(
        a.on_key(key(KeyCode::Backspace)),
        [Action::List {
            side: LOCAL,
            path: RemotePath::new("/home"),
            focus: Some("me".into())
        }]
    );
    // Enter on a file does nothing; Enter on ".." goes up.
    press(&mut a, "G");
    assert!(a.on_key(key(KeyCode::Enter)).is_empty());
    press(&mut a, "g");
    assert_eq!(a.on_key(key(KeyCode::Enter)).len(), 1);
}

#[test]
fn selection_and_copy_targets() {
    let mut a = app();
    press(&mut a, "j "); // select "bin", cursor moves on
    press(&mut a, " ");
    assert_eq!(a.pane().selected.len(), 2);
    press(&mut a, " ");
    press(&mut a, "k ");
    assert_eq!(a.pane().selected.len(), 2);
    press(&mut a, "aa");
    assert!(a.pane().selected.is_empty()); // all, then none
    press(&mut a, "a");
    assert_eq!(a.pane().selected.len(), 4);
    let acts = press(&mut a, "c");
    let [Action::Copy {
        from,
        sources,
        dest,
    }] = &acts[..]
    else {
        panic!("{acts:?}")
    };
    assert_eq!((*from, sources.len()), (LOCAL, 4));
    assert_eq!(dest, &RemotePath::new("/srv"));
    assert!(a.pane().selected.is_empty());
}

#[test]
fn copy_without_selection_uses_cursor_and_skips_up_entry() {
    let mut a = app();
    assert!(press(&mut a, "c").is_empty()); // on ".."
    assert!(a.status_err);
    press(&mut a, "j");
    assert_eq!(press(&mut a, "c").len(), 1);
}

#[test]
fn copy_blocked_while_transfer_runs() {
    let mut a = app();
    a.start_transfer(Control::new(), REMOTE);
    press(&mut a, "j");
    assert!(press(&mut a, "c").is_empty());
    assert!(a.status.contains("already"));
}

#[test]
fn filter_mode_narrows_and_escape_clears() {
    let mut a = app();
    press(&mut a, "/AL");
    assert!(a.filtering);
    assert_eq!(names(a.pane()), ["..", "Alpha.log"]);
    a.on_key(key(KeyCode::Backspace));
    assert_eq!(names(a.pane()), ["..", "Alpha.log", "zeta.txt"]);
    press(&mut a, "q"); // text while filtering, not quit
    assert!(!a.quit);
    assert_eq!(names(a.pane()), [".."]);
    a.on_key(key(KeyCode::Enter));
    assert!(!a.filtering);
    assert_eq!(a.pane().filter, "Aq");
    press(&mut a, "/");
    a.on_key(key(KeyCode::Esc));
    assert!(a.pane().filter.is_empty());
    assert_eq!(names(a.pane()).len(), 5);
}

#[test]
fn mkdir_popup_flow() {
    let mut a = app();
    press(&mut a, "m");
    assert!(matches!(a.popup, Some(Popup::Input { .. })));
    assert!(a.on_key(key(KeyCode::Enter)).is_empty()); // empty name rejected, popup stays
    assert!(a.popup.is_some());
    press(&mut a, "new");
    a.on_key(key(KeyCode::Backspace));
    press(&mut a, "w");
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        [Action::Mkdir {
            side: LOCAL,
            path: RemotePath::new("/home/me/new")
        }]
    );
    assert!(a.popup.is_none());
    press(&mut a, "m");
    a.on_key(key(KeyCode::Esc));
    assert!(a.popup.is_none());
}

#[test]
fn rename_popup_prefills_name() {
    let mut a = app();
    press(&mut a, "jr");
    let Some(Popup::Input { text, .. }) = &a.popup else {
        panic!()
    };
    assert_eq!(text, "bin");
    press(&mut a, "2");
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        [Action::Rename {
            side: LOCAL,
            from: RemotePath::new("/home/me/bin"),
            to: RemotePath::new("/home/me/bin2")
        }]
    );
}

#[test]
fn delete_needs_confirmation() {
    let mut a = app();
    press(&mut a, "jd");
    assert!(matches!(a.popup, Some(Popup::Confirm { .. })));
    assert!(press(&mut a, "n").is_empty());
    assert!(a.popup.is_none());
    press(&mut a, "d");
    assert_eq!(
        press(&mut a, "y"),
        [Action::Delete {
            side: LOCAL,
            paths: vec![RemotePath::new("/home/me/bin")]
        }]
    );
}

#[test]
fn help_popup_and_quit() {
    let mut a = app();
    press(&mut a, "?");
    assert_eq!(a.popup, Some(Popup::Help));
    press(&mut a, "q"); // any key closes help, does not quit
    assert!(a.popup.is_none() && !a.quit);
    a.on_key(key(KeyCode::F(10)));
    assert!(a.quit);
}

#[test]
fn pause_resume_and_cancel() {
    let mut a = app();
    let c = Control::new();
    a.start_transfer(c.clone(), REMOTE);
    press(&mut a, "p");
    assert!(c.is_paused());
    press(&mut a, "p");
    assert!(!c.is_paused());
    a.on_key(key(KeyCode::Esc));
    assert!(c.is_cancelled());
}

#[test]
fn transfer_events_drive_progress_and_refresh() {
    let mut a = app();
    a.start_transfer(Control::new(), REMOTE);
    let src = RemotePath::new("/a/f.bin");
    for ev in [
        Event::Planned {
            files: 2,
            bytes: 100,
        },
        Event::Started {
            id: 0,
            src: src.clone(),
            dst: src.clone(),
            size: 50,
        },
        Event::Progress {
            id: 0,
            done: 25,
            total: 50,
        },
    ] {
        assert!(a.on_transfer_event(ev).is_none());
    }
    let t = a.transfer.as_ref().unwrap();
    assert_eq!(t.done_bytes(), 25);
    assert!((t.ratio() - 0.25).abs() < 1e-9);
    a.on_transfer_event(Event::Finished { id: 0, bytes: 50 });
    a.on_transfer_event(Event::Failed {
        id: 1,
        error: "boom".into(),
    });
    let act = a.on_transfer_event(Event::Done {
        completed: 1,
        skipped: 0,
        failed: 1,
        bytes: 50,
        cancelled: false,
    });
    assert!(matches!(act, Some(Action::List { side: REMOTE, .. })));
    let t = a.transfer.as_ref().unwrap();
    assert!(!t.running());
    assert!(t.log[0].contains("boom"));
    assert!(a.status.contains("1 failed"));
}

#[test]
fn renders_panes_transfer_and_popups() {
    let mut a = app();
    a.panes[0].selected.insert("docs".into());
    a.start_transfer(Control::new(), REMOTE);
    let src = RemotePath::new("/a/big.iso");
    a.on_transfer_event(Event::Planned {
        files: 1,
        bytes: 2048,
    });
    a.on_transfer_event(Event::Started {
        id: 0,
        src: src.clone(),
        dst: src,
        size: 2048,
    });
    a.on_transfer_event(Event::Progress {
        id: 0,
        done: 1024,
        total: 2048,
    });
    a.on_transfer_event(Event::Failed {
        id: 7,
        error: "denied".into(),
    });

    let draw = |a: &App| {
        let mut t = Terminal::new(TestBackend::new(110, 30)).unwrap();
        t.draw(|f| ui::draw(f, a)).unwrap();
        t.backend().to_string()
    };
    let s = draw(&a);
    for want in [
        "local",
        "user@host",
        "/home/me",
        "/srv",
        "Name",
        "Size",
        "Modified",
        "..",
        "*docs/",
        "Alpha.log",
        "zeta.txt",
        "<DIR>",
        "4.9K",
        "1970-01-01 00:05",
        "Transfer (running)",
        "50%",
        "big.iso",
        "1/",
        "failed #7: denied",
    ] {
        assert!(s.contains(want), "missing {want:?} in:\n{s}");
    }
    assert!(!s.contains(".hidden"));

    a.popup = Some(Popup::Help);
    assert!(draw(&a).contains("switch pane"));
    a.popup = Some(Popup::Input {
        kind: InputKind::Mkdir,
        text: "abc".into(),
    });
    let s = draw(&a);
    assert!(s.contains("New directory") && s.contains("abc_"));
    a.popup = Some(Popup::Confirm {
        paths: vec![RemotePath::new("/x/gone.txt")],
    });
    let s = draw(&a);
    assert!(s.contains("Delete 1 item(s)?") && s.contains("gone.txt"));
}

#[test]
fn formats_sizes_and_durations() {
    assert_eq!(ui::human_size(0), "0B");
    assert_eq!(ui::human_size(1536), "1.5K");
    assert_eq!(ui::human_duration(Duration::from_secs(75)), "1m15s");
    assert_eq!(ui::human_duration(Duration::from_secs(3700)), "1h01m");
}
