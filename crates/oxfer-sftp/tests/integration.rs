//! Integration tests against a throwaway unprivileged `sshd`.
//! Skipped (with a message) when sshd / sftp-server are not available.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use oxfer_core::{Error, RemoteFs, RemotePath};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, SftpFs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    dir: tempdir::Dir,
    child: Child,
    port: u16,
}

mod tempdir {
    use std::path::{Path, PathBuf};
    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("oxfer-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|p| p.exists())
}

fn keygen(path: &Path) {
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ssh-keygen failed");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Server {
    fn start(tag: &str) -> Option<Self> {
        let sshd = find(&["/usr/sbin/sshd", "/usr/bin/sshd"])?;
        let sftp = find(&[
            "/usr/libexec/sftp-server",
            "/usr/lib/openssh/sftp-server",
            "/usr/lib/ssh/sftp-server",
        ])?;
        let dir = tempdir::Dir::new(tag);
        let d = dir.path();
        keygen(&d.join("host"));
        keygen(&d.join("client"));
        std::fs::copy(d.join("client.pub"), d.join("authorized_keys")).unwrap();
        let port = free_port();
        let conf = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host\nPidFile {d}/pid\n\
             AuthorizedKeysFile {d}/authorized_keys\nStrictModes no\nUsePAM no\n\
             PasswordAuthentication no\nPubkeyAuthentication yes\n\
             Subsystem sftp {sftp}\n",
            d = d.display(),
            sftp = sftp.display()
        );
        std::fs::write(d.join("sshd_config"), conf).unwrap();
        let child = Command::new(sshd)
            .args(["-D", "-e", "-f"])
            .arg(d.join("sshd_config"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // wait for the port to accept connections
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(Server { dir, child, port });
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }

    fn opts(&self, decision: HostKeyDecision) -> ConnectOptions {
        let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        let mut o = ConnectOptions::new(
            "127.0.0.1",
            user,
            Auth::Key {
                path: self.dir.path().join("client"),
                passphrase: None,
            },
        );
        o.port = self.port;
        o.known_hosts = Some(self.dir.path().join("known_hosts"));
        o.verifier = Arc::new(move |_| decision);
        o
    }

    /// Scratch directory the tests can use as the remote working area.
    fn work(&self) -> RemotePath {
        let w = self.dir.path().join("work");
        std::fs::create_dir_all(&w).unwrap();
        RemotePath::new(w.to_string_lossy().to_string())
    }
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
