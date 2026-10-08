//! Runs `oxfer tui` inside a real pseudo-terminal, drives it with key presses
//! and inspects the rendered screen through a terminal emulator.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oxfer_testutil::Server;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

struct Term {
    parser: Arc<Mutex<vt100::Parser>>,
}

impl Term {
    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&self, what: &str, timeout: Duration) {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.screen().contains(what) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what:?}; screen:\n{}", self.screen());
    }
}

#[test]
fn tui_browses_copies_and_quits() {
    let Some(srv) = Server::start("tui") else {
        eprintln!("skipping: sshd not available");
        return;
    };
    let work = srv.work();
    std::fs::write(work.join("remote-marker.txt"), "r").unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(local.path().join("upload-me.txt"), "payload").unwrap();

    let bin = env!("CARGO_BIN_EXE_oxfer");
    let target = format!("{}@127.0.0.1:{}", srv.user(), work.display());
    let common = |c: &mut std::process::Command| {
        c.args(["-P", &srv.port.to_string(), "-i"])
            .arg(srv.client_key())
            .args(["--ssh-config", "/nonexistent", "--known-hosts"])
            .arg(srv.known_hosts())
            .env("SSH_AUTH_SOCK", "");
    };

    // record the host key first so the TUI itself never prompts
    let mut ls = std::process::Command::new(bin);
    common(&mut ls);
    let mut child = ls
        .args(["ls", &target])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
    assert!(child.wait().unwrap().success());

    let pty = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 140,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(bin);
    cmd.args(["-P", &srv.port.to_string(), "-i"]);
    cmd.arg(srv.client_key());
    cmd.args(["--ssh-config", "/nonexistent", "--known-hosts"]);
    cmd.arg(srv.known_hosts());
    cmd.args(["tui", &target]);
    cmd.env("SSH_AUTH_SOCK", "");
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(local.path());
    let mut child = pty.slave.spawn_command(cmd).unwrap();
    drop(pty.slave);

    let parser = Arc::new(Mutex::new(vt100::Parser::new(40, 140, 0)));
    let mut reader = pty.master.try_clone_reader().unwrap();
    let p2 = parser.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&buf[..n]);
        }
    });
    let mut writer = pty.master.take_writer().unwrap();
    let term = Term { parser };

    // both panes render their listings
    term.wait_for("upload-me.txt", Duration::from_secs(15));
    term.wait_for("remote-marker.txt", Duration::from_secs(15));

    // help popup opens and closes
    writer.write_all(b"?").unwrap();
    term.wait_for("Help", Duration::from_secs(5));
    writer.write_all(b"\x1b").unwrap();
    std::thread::sleep(Duration::from_millis(200));

    // select the local file (cursor starts on "..") and copy it to the remote pane
    writer.write_all(b"j ").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    writer.write_all(b"c").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while !work.join("upload-me.txt").exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(work.join("upload-me.txt")).unwrap_or_default(),
        "payload",
        "screen:\n{}",
        term.screen()
    );

    // quit cleanly
    writer.write_all(b"q").unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            Instant::now() < end,
            "tui did not exit; screen:\n{}",
            term.screen()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
}
