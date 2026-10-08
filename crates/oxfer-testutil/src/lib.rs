//! Test helpers: a throwaway unprivileged `sshd` with SFTP enabled.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let p = std::env::temp_dir().join(format!("oxfer-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Server {
    pub dir: TempDir,
    child: Child,
    pub port: u16,
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

pub fn keygen(path: &Path) {
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .expect("ssh-keygen")
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
    /// Start sshd on a free local port, or `None` if sshd/sftp-server are missing.
    pub fn start(tag: &str) -> Option<Self> {
        let sshd = find(&["/usr/sbin/sshd", "/usr/bin/sshd"])?;
        let sftp = find(&[
            "/usr/libexec/sftp-server",
            "/usr/lib/openssh/sftp-server",
            "/usr/lib/ssh/sftp-server",
        ])?;
        let dir = TempDir::new(tag);
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
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(Server { dir, child, port });
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }

    pub fn user(&self) -> String {
        std::env::var("USER").unwrap_or_else(|_| "root".into())
    }

    pub fn client_key(&self) -> PathBuf {
        self.dir.path().join("client")
    }

    pub fn known_hosts(&self) -> PathBuf {
        self.dir.path().join("known_hosts")
    }

    /// A fresh scratch directory (on the shared local disk) usable as the remote working area.
    pub fn work(&self) -> PathBuf {
        let w = self.dir.path().join("work");
        std::fs::create_dir_all(&w).unwrap();
        w
    }
}
