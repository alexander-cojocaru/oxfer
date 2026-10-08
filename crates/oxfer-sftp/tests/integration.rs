//! Integration tests against a throwaway unprivileged `sshd`.
//! Skipped (with a message) when sshd / sftp-server are not available.

use std::sync::Arc;

use oxfer_core::{Error, RemoteFs, RemotePath};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, SftpFs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server(oxfer_testutil::Server);

impl std::ops::Deref for Server {
    type Target = oxfer_testutil::Server;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Server {
    fn start(tag: &str) -> Option<Self> {
        oxfer_testutil::Server::start(tag).map(Server)
    }

    fn opts(&self, decision: HostKeyDecision) -> ConnectOptions {
        let mut o = ConnectOptions::new(
            "127.0.0.1",
            self.user(),
            Auth::Key {
                path: self.client_key(),
                passphrase: None,
            },
        );
        o.port = self.port;
        o.known_hosts = Some(self.known_hosts());
        o.verifier = Arc::new(move |_| decision);
        o
    }

    fn work(&self) -> RemotePath {
        RemotePath::new(self.0.work().to_string_lossy().to_string())
    }
}

fn keygen(p: &std::path::Path) {
    oxfer_testutil::keygen(p)
}

macro_rules! server {
    ($tag:expr) => {
        match Server::start($tag) {
            Some(s) => s,
            None => {
                eprintln!("skipping: sshd not available");
                return;
            }
        }
    };
}

#[tokio::test]
async fn file_operations_roundtrip() {
    let srv = server!("ops");
    let fs = SftpFs::connect(srv.opts(HostKeyDecision::AcceptAndSave))
        .await
        .unwrap();
    let work = srv.work();

    // upload
    let file = work.join("a.txt");
    let mut w = fs.open_write(&file, 0).await.unwrap();
    w.write_all(b"hello world").await.unwrap();
    w.shutdown().await.unwrap();
    drop(w);

    // stat + list
    let e = fs.stat(&file).await.unwrap();
    assert_eq!(e.size, 11);
    assert!(!e.is_dir());
    let names: Vec<_> = fs
        .list(&work)
        .await
        .unwrap()
        .iter()
        .map(|e| e.name().to_string())
        .collect();
    assert_eq!(names, ["a.txt"]);

    // ranged read (resume)
    let mut r = fs.open_read(&file, 6).await.unwrap();
    let mut buf = String::new();
    r.read_to_string(&mut buf).await.unwrap();
    assert_eq!(buf, "world");

    // append at offset
    let mut w = fs.open_write(&file, 11).await.unwrap();
    w.write_all(b"!!").await.unwrap();
    w.shutdown().await.unwrap();
    drop(w);
    assert_eq!(
        std::fs::read_to_string(file.as_str()).unwrap(),
        "hello world!!"
    );

    // mkdir / rename / remove
    let dir = work.join("sub");
    fs.mkdir(&dir).await.unwrap();
    assert!(fs.stat(&dir).await.unwrap().is_dir());
    let moved = work.join("b.txt");
    fs.rename(&file, &moved).await.unwrap();
    assert!(matches!(fs.stat(&file).await, Err(Error::NotFound(_))));
    fs.remove(&moved).await.unwrap();
    fs.remove(&dir).await.unwrap();
    assert!(fs.list(&work).await.unwrap().is_empty());
}

#[tokio::test]
async fn not_found_error() {
    let srv = server!("nf");
    let fs = SftpFs::connect(srv.opts(HostKeyDecision::AcceptOnce))
        .await
        .unwrap();
    let missing = srv.work().join("nope");
    assert!(matches!(fs.stat(&missing).await, Err(Error::NotFound(_))));
}

#[tokio::test]
async fn unknown_host_rejected_by_default() {
    let srv = server!("rej");
    let err = SftpFs::connect(srv.opts(HostKeyDecision::Reject))
        .await
        .err()
        .expect("must fail");
    assert!(matches!(err, Error::HostKey(_)), "{err:?}");
}

#[tokio::test]
async fn host_key_saved_then_changed_is_refused() {
    let srv = server!("hk");
    // first connection saves the key
    SftpFs::connect(srv.opts(HostKeyDecision::AcceptAndSave))
        .await
        .unwrap();
    // second succeeds without asking (verifier would reject)
    SftpFs::connect(srv.opts(HostKeyDecision::Reject))
        .await
        .unwrap();

    // corrupt known_hosts with a different key for the same host
    let kh = srv.dir.path().join("known_hosts");
    let other = srv.dir.path().join("other");
    keygen(&other);
    let pubkey = std::fs::read_to_string(srv.dir.path().join("other.pub")).unwrap();
    let mut parts = pubkey.split_whitespace();
    let (alg, key) = (parts.next().unwrap(), parts.next().unwrap());
    std::fs::write(&kh, format!("[127.0.0.1]:{} {alg} {key}\n", srv.port)).unwrap();

    let err = SftpFs::connect(srv.opts(HostKeyDecision::AcceptAndSave))
        .await
        .err()
        .expect("must fail");
    assert!(
        matches!(&err, Error::HostKey(m) if m.contains("CHANGED")),
        "{err:?}"
    );
}

#[tokio::test]
async fn wrong_key_is_auth_error() {
    let srv = server!("auth");
    let mut o = srv.opts(HostKeyDecision::AcceptOnce);
    let bad = srv.dir.path().join("bad");
    keygen(&bad);
    o.auth = Auth::Key {
        path: bad,
        passphrase: None,
    };
    let err = SftpFs::connect(o).await.err().expect("must fail");
    assert!(matches!(err, Error::Auth(_)), "{err:?}");
}

#[tokio::test]
async fn connects_through_a_jump_host() {
    let srv = server!("jump");
    // jump through the server to itself (loopback), then talk SFTP over the tunnel
    let mut target = srv.opts(HostKeyDecision::AcceptAndSave);
    target.jump = Some(Box::new(srv.opts(HostKeyDecision::AcceptAndSave)));
    let fs = SftpFs::connect(target).await.unwrap();
    let work = srv.work();
    std::fs::write(srv.0.work().join("via-jump.txt"), "x").unwrap();
    let names: Vec<_> = fs
        .list(&work)
        .await
        .unwrap()
        .iter()
        .map(|e| e.name().to_string())
        .collect();
    assert!(names.contains(&"via-jump.txt".to_string()), "{names:?}");
}

#[tokio::test]
async fn unreachable_target_through_jump_is_connection_error() {
    let srv = server!("jump-bad");
    let mut target = srv.opts(HostKeyDecision::AcceptOnce);
    target.port = free_closed_port();
    target.jump = Some(Box::new(srv.opts(HostKeyDecision::AcceptOnce)));
    let err = SftpFs::connect(target).await.err().expect("must fail");
    assert!(
        matches!(err, Error::Connection(ref m) if m.contains("jump host")),
        "{err:?}"
    );
}

fn free_closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
