//! Shared in-process mock FTP / FTPS server.
#![allow(dead_code)]

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oxfer_core::{RemoteFs, RemotePath};
use oxfer_ftp::{FtpConnectOptions, FtpFs, TlsMode};
use tokio::fs::OpenOptions;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader,
    ReadHalf, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

#[derive(Clone, Copy)]
pub struct Flavor {
    pub mlsd: bool,
    pub epsv: bool,
}

/// How the mock secures its control connection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Security {
    Plain,
    /// `AUTH TLS` upgrade, then `PBSZ`/`PROT P` for data connections.
    Explicit,
    /// TLS from the first byte; data connections are always TLS.
    Implicit,
}

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = Box<dyn Io>;

pub struct Mock {
    pub dir: PathBuf,
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

impl Mock {
    pub async fn start(flavor: Flavor) -> Mock {
        Self::start_secure(flavor, Security::Plain, None).await
    }

    pub async fn start_secure(
        flavor: Flavor,
        security: Security,
        tls: Option<Arc<ServerConfig>>,
    ) -> Mock {
        let acceptor = tls.map(TlsAcceptor::from);
        assert_eq!(security == Security::Plain, acceptor.is_none());
        let dir = std::env::temp_dir().join(format!(
            "oxfer-ftp-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = dir.clone();
        let task = tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(session(
                    sock,
                    root.clone(),
                    flavor,
                    security,
                    acceptor.clone(),
                ));
            }
        });
        Mock { dir, port, task }
    }

    pub fn opts(&self) -> FtpConnectOptions {
        let mut o = FtpConnectOptions::new("127.0.0.1", "user", "secret");
        o.port = self.port;
        o.tls = TlsMode::None;
        o
    }
}

type Ctl = WriteHalf<Stream>;

async fn reply(w: &mut Ctl, s: &str) {
    w.write_all(format!("{s}\r\n").as_bytes()).await.unwrap();
}

fn host_path(root: &Path, arg: &str) -> PathBuf {
    root.join(arg.trim_start_matches('/'))
}

fn mlsx_facts(md: &std::fs::Metadata) -> String {
    let kind = if md.is_dir() { "dir" } else { "file" };
    format!("type={kind};size={};modify=20240105123456;", md.len())
}

fn unix_line(md: &std::fs::Metadata, name: &str) -> String {
    let d = if md.is_dir() { 'd' } else { '-' };
    format!("{d}rwxr-xr-x 1 ftp ftp {} Jan  5 12:34 {name}", md.len())
}

async fn accept_data(l: TcpListener, tls: &Option<TlsAcceptor>, secure: bool) -> Stream {
    let (data, _) = l.accept().await.unwrap();
    match tls {
        Some(a) if secure => Box::new(a.accept(data).await.unwrap()),
        _ => Box::new(data),
    }
}

async fn session(
    sock: TcpStream,
    root: PathBuf,
    flavor: Flavor,
    security: Security,
    tls: Option<TlsAcceptor>,
) {
    let sock: Stream = if security == Security::Implicit {
        match tls.as_ref().unwrap().accept(sock).await {
            Ok(s) => Box::new(s),
            // Client rejected our certificate (or hung up): nothing to serve.
            Err(_) => return,
        }
    } else {
        Box::new(sock)
    };
    let mut data_secure = security == Security::Implicit;
    let (r, mut w): (ReadHalf<Stream>, Ctl) = tokio::io::split(sock);
    let mut r = BufReader::new(r);
    reply(&mut w, "220 mock ready").await;
    let mut pasv: Option<TcpListener> = None;
    let mut rest = 0u64;
    let mut rnfr: Option<PathBuf> = None;
    let mut user_ok = false;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
        let cmd = cmd.to_ascii_uppercase();
        match cmd.as_str() {
            "USER" => {
                user_ok = arg == "user";
                reply(&mut w, "331 password please").await
            }
            "PASS" if user_ok && arg == "secret" => reply(&mut w, "230 logged in").await,
            "PASS" => reply(&mut w, "530 Login incorrect.").await,
            "FEAT" => {
                let mut s = String::from("211-Features:\r\n UTF8\r\n REST STREAM\r\n");
                if flavor.epsv {
                    s.push_str(" EPSV\r\n");
                }
                if flavor.mlsd {
                    s.push_str(" MLST type*;size*;modify*;\r\n");
                }
                s.push_str("211 End");
                reply(&mut w, &s).await
            }
            "AUTH" if security == Security::Explicit && arg.eq_ignore_ascii_case("TLS") => {
                reply(&mut w, "234 AUTH TLS ok").await;
                let sock = r.into_inner().unsplit(w);
                let Ok(s) = tls.as_ref().unwrap().accept(sock).await else {
                    return;
                };
                let (nr, nw) = tokio::io::split(Box::new(s) as Stream);
                r = BufReader::new(nr);
                w = nw;
            }
            "PBSZ" if security != Security::Plain => reply(&mut w, "200 PBSZ=0").await,
            "PROT" if security != Security::Plain => {
                data_secure = arg.eq_ignore_ascii_case("P");
                reply(&mut w, "200 PROT ok").await
            }
            "OPTS" | "TYPE" | "NOOP" => reply(&mut w, "200 ok").await,
            "EPSV" | "PASV" => {
                if cmd == "EPSV" && !flavor.epsv {
                    reply(&mut w, "500 EPSV not understood").await;
                    continue;
                }
                let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let p = l.local_addr().unwrap().port();
                pasv = Some(l);
                if cmd == "EPSV" {
                    reply(
                        &mut w,
                        &format!("229 Entering Extended Passive Mode (|||{p}|)"),
                    )
                    .await
                } else {
                    reply(
                        &mut w,
                        &format!(
                            "227 Entering Passive Mode (127,0,0,1,{},{})",
                            p / 256,
                            p % 256
                        ),
                    )
                    .await
                }
            }
            "REST" => {
                rest = arg.parse().unwrap();
                reply(&mut w, "350 restarting").await
            }
            "SIZE" => match std::fs::metadata(host_path(&root, arg)) {
                Ok(m) if m.is_file() => reply(&mut w, &format!("213 {}", m.len())).await,
                _ => reply(&mut w, "550 No such file or directory.").await,
            },
            "MLST" if flavor.mlsd => match std::fs::metadata(host_path(&root, arg)) {
                Ok(m) => {
                    reply(
                        &mut w,
                        &format!("250-Listing {arg}\r\n {} {arg}\r\n250 End", mlsx_facts(&m)),
                    )
                    .await
                }
                Err(_) => reply(&mut w, "550 No such file or directory.").await,
            },
            "MLSD" | "LIST" => {
                if cmd == "MLSD" && !flavor.mlsd {
                    reply(&mut w, "500 Unknown command").await;
                    continue;
                }
                let dir = host_path(&root, arg);
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    reply(&mut w, "550 No such file or directory.").await;
                    continue;
                };
                let mut entries: Vec<_> = rd.map(|e| e.unwrap()).collect();
                entries.sort_by_key(|e| e.file_name());
                let mut out = String::new();
                if cmd == "LIST" {
                    out.push_str("total 0\r\n");
                    out.push_str("drwxr-xr-x 1 ftp ftp 0 Jan  5 12:34 .\r\n");
                }
                for e in entries {
                    let md = e.metadata().unwrap();
                    let name = e.file_name().to_string_lossy().to_string();
                    if cmd == "MLSD" {
                        out.push_str(&format!("{} {name}\r\n", mlsx_facts(&md)));
                    } else {
                        out.push_str(&format!("{}\r\n", unix_line(&md, &name)));
                    }
                }
                reply(&mut w, "150 here comes the listing").await;
                let mut data = accept_data(pasv.take().unwrap(), &tls, data_secure).await;
                data.write_all(out.as_bytes()).await.unwrap();
                let _ = data.shutdown().await;
                drop(data);
                reply(&mut w, "226 done").await;
            }
            "RETR" => {
                let path = host_path(&root, arg);
                let Ok(mut f) = tokio::fs::File::open(&path).await else {
                    reply(&mut w, "550 No such file or directory.").await;
                    continue;
                };
                f.seek(SeekFrom::Start(std::mem::take(&mut rest)))
                    .await
                    .unwrap();
                reply(&mut w, "150 sending").await;
                let mut data = accept_data(pasv.take().unwrap(), &tls, data_secure).await;
                match tokio::io::copy(&mut f, &mut data).await {
                    Ok(_) => {
                        let _ = data.shutdown().await;
                        drop(data);
                        reply(&mut w, "226 done").await
                    }
                    Err(_) => {
                        drop(data);
                        reply(&mut w, "426 Connection closed; transfer aborted.").await
                    }
                }
            }
            "STOR" | "APPE" => {
                let path = host_path(&root, arg);
                let off = std::mem::take(&mut rest);
                let mut oo = OpenOptions::new();
                oo.write(true).create(true);
                if cmd == "APPE" {
                    oo.append(true);
                } else if off == 0 {
                    oo.truncate(true);
                }
                let Ok(mut f) = oo.open(&path).await else {
                    reply(&mut w, "553 Could not create file.").await;
                    continue;
                };
                if cmd == "STOR" && off > 0 {
                    f.seek(SeekFrom::Start(off)).await.unwrap();
                }
                reply(&mut w, "150 receiving").await;
                let mut data = accept_data(pasv.take().unwrap(), &tls, data_secure).await;
                let _ = tokio::io::copy(&mut data, &mut f).await;
                f.flush().await.unwrap();
                reply(&mut w, "226 done").await;
            }
            "MKD" => {
                let p = host_path(&root, arg);
                match std::fs::create_dir(&p) {
                    Ok(()) => reply(&mut w, &format!("257 \"{arg}\" created")).await,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        reply(&mut w, "550 Create directory operation failed.").await
                    }
                    Err(_) => reply(&mut w, "550 Create directory operation failed.").await,
                }
            }
            "DELE" => match std::fs::remove_file(host_path(&root, arg)) {
                Ok(()) => reply(&mut w, "250 deleted").await,
                Err(_) => reply(&mut w, "550 Delete operation failed.").await,
            },
            "RMD" => match std::fs::remove_dir(host_path(&root, arg)) {
                Ok(()) => reply(&mut w, "250 removed").await,
                Err(_) => reply(&mut w, "550 Remove directory operation failed.").await,
            },
            "RNFR" => {
                let p = host_path(&root, arg);
                if p.exists() {
                    rnfr = Some(p);
                    reply(&mut w, "350 ready for RNTO").await
                } else {
                    reply(&mut w, "550 No such file or directory.").await
                }
            }
            "RNTO" => match rnfr.take() {
                Some(from) if std::fs::rename(&from, host_path(&root, arg)).is_ok() => {
                    reply(&mut w, "250 renamed").await
                }
                _ => reply(&mut w, "550 Rename failed.").await,
            },
            "QUIT" => {
                reply(&mut w, "221 bye").await;
                return;
            }
            _ => reply(&mut w, "502 not implemented").await,
        }
    }
}

pub const FLAVORS: [Flavor; 3] = [
    Flavor {
        mlsd: true,
        epsv: true,
    },
    Flavor {
        mlsd: false,
        epsv: true,
    },
    Flavor {
        mlsd: false,
        epsv: false,
    },
];

pub fn rp(s: &str) -> RemotePath {
    RemotePath::new(s)
}

pub fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

pub async fn put(fs: &FtpFs, path: &str, offset: u64, data: &[u8]) {
    let mut w = fs.open_write(&rp(path), offset).await.unwrap();
    w.write_all(data).await.unwrap();
    w.shutdown().await.unwrap();
}

pub async fn get(fs: &FtpFs, path: &str, offset: u64) -> Vec<u8> {
    let mut r = fs.open_read(&rp(path), offset).await.unwrap();
    let mut out = Vec::new();
    r.read_to_end(&mut out).await.unwrap();
    out
}
