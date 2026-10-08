//! End-to-end tests of the `oxfer` binary against a throwaway sshd.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use oxfer_testutil::Server;

struct Env {
    srv: Server,
}

fn env(tag: &str) -> Option<Env> {
    Server::start(tag).map(|srv| Env { srv })
}

impl Env {
    fn run(&self, args: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_oxfer"));
        c.args(["-P", &self.srv.port.to_string(), "-i"])
            .arg(self.srv.client_key())
            .arg("--known-hosts")
            .arg(self.srv.known_hosts())
            .args(args)
            .env("SSH_AUTH_SOCK", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        // answer the host key prompt
        child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
        child.wait_with_output().unwrap()
    }

    fn host(&self, path: &std::path::Path) -> String {
        format!("{}@127.0.0.1:{}", self.srv.user(), path.display())
    }
}

macro_rules! setup {
    ($tag:expr) => {
        match env($tag) {
            Some(e) => e,
            None => {
                eprintln!("skipping: sshd not available");
                return;
            }
        }
    };
}

fn ok(o: &Output) {
    assert!(
        o.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn recursive_roundtrip_ls_mv_rm() {
    let e = setup!("cli");
    let work = e.srv.work();
    let local = tempfile::tempdir().unwrap();
    let tree = local.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a.txt"), "alpha").unwrap();
    std::fs::write(tree.join("sub/b.bin"), vec![3u8; 300_000]).unwrap();

    // upload the tree
    let o = e.run(&[
        "put",
        "-r",
        "--verify",
        tree.to_str().unwrap(),
        &e.host(&work),
    ]);
    ok(&o);
    assert_eq!(std::fs::read(work.join("tree/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(work.join("tree/sub/b.bin")).unwrap().len(),
        300_000
    );

    // ls
    let o = e.run(&["ls", &e.host(&work.join("tree"))]);
    ok(&o);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("a.txt") && out.contains("sub"), "{out}");

    // download it again under a new name
    let back = local.path().join("back");
    let o = e.run(&[
        "get",
        "-r",
        &e.host(&work.join("tree")),
        back.to_str().unwrap(),
    ]);
    ok(&o);
    assert_eq!(std::fs::read(back.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(back.join("sub/b.bin")).unwrap().len(),
        300_000
    );

    // a directory without -r is refused
    let o = e.run(&["get", &e.host(&work.join("tree")), back.to_str().unwrap()]);
    assert!(!o.status.success());

    // mv, mkdir, rm -r
    let o = e.run(&[
        "mv",
        &e.host(&work.join("tree")),
        &e.host(&work.join("moved")),
    ]);
    ok(&o);
    assert!(work.join("moved/a.txt").exists() && !work.join("tree").exists());
    ok(&e.run(&["mkdir", &e.host(&work.join("newdir"))]));
    assert!(work.join("newdir").is_dir());
    ok(&e.run(&["rm", "-r", &e.host(&work.join("moved"))]));
    assert!(!work.join("moved").exists());
}

#[test]
fn rejects_unknown_host_when_not_confirmed() {
    let e = setup!("cli-reject");
    let mut c = Command::new(env!("CARGO_BIN_EXE_oxfer"));
    c.args(["-P", &e.srv.port.to_string(), "-i"])
        .arg(e.srv.client_key())
        .arg("--known-hosts")
        .arg(e.srv.known_hosts())
        .arg("ls")
        .arg(e.host(&e.srv.work()))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"no\n").unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("host key"));
}
