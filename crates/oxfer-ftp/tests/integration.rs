//! Integration tests against a minimal in-process mock FTP server (no external
//! tools needed, so they are deterministic and never skipped). The mock serves
//! a temp directory and is run in several flavours: with/without MLSD/MLST and
//! with/without EPSV, to exercise the fallbacks.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oxfer_core::{EntryKind, Error, RemoteFs, RemotePath};
use oxfer_ftp::{FtpConnectOptions, FtpFs, TlsMode};
use tokio::fs::OpenOptions;
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy)]
struct Flavor {
    mlsd: bool,
    epsv: bool,
}

struct Mock {
    dir: PathBuf,
    port: u16,
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
    async fn start(flavor: Flavor) -> Mock {
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
                tokio::spawn(session(sock, root.clone(), flavor));
            }
        });
        Mock { dir, port, task }
    }

    fn opts(&self) -> FtpConnectOptions {
        let mut o = FtpConnectOptions::new("127.0.0.1", "user", "secret");
        o.port = self.port;
        o.tls = TlsMode::None;
        o
    }
}

type Ctl = WriteHalf<TcpStream>;

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

async fn session(sock: TcpStream, root: PathBuf, flavor: Flavor) {
    let (r, mut w): (ReadHalf<TcpStream>, Ctl) = tokio::io::split(sock);
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
                let (mut data, _) = pasv.take().unwrap().accept().await.unwrap();
                data.write_all(out.as_bytes()).await.unwrap();
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
                let (mut data, _) = pasv.take().unwrap().accept().await.unwrap();
                match tokio::io::copy(&mut f, &mut data).await {
                    Ok(_) => {
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
                let (mut data, _) = pasv.take().unwrap().accept().await.unwrap();
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

const FLAVORS: [Flavor; 3] = [
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

fn rp(s: &str) -> RemotePath {
    RemotePath::new(s)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

async fn put(fs: &FtpFs, path: &str, offset: u64, data: &[u8]) {
    let mut w = fs.open_write(&rp(path), offset).await.unwrap();
    w.write_all(data).await.unwrap();
    w.shutdown().await.unwrap();
}

async fn get(fs: &FtpFs, path: &str, offset: u64) -> Vec<u8> {
    let mut r = fs.open_read(&rp(path), offset).await.unwrap();
    let mut out = Vec::new();
    r.read_to_end(&mut out).await.unwrap();
    out
}

#[tokio::test]
async fn full_session_for_every_flavor() {
    for flavor in FLAVORS {
        let mock = Mock::start(flavor).await;
        let fs = FtpFs::connect(mock.opts()).await.unwrap();
        std::fs::write(mock.dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(mock.dir.join("sp ace.txt"), b"x").unwrap();
        std::fs::create_dir(mock.dir.join("dir with space")).unwrap();

        // list + stat
        let mut entries = fs.list(&RemotePath::root()).await.unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let names: Vec<_> = entries.iter().map(|e| e.name().to_string()).collect();
        assert_eq!(names, ["a.txt", "dir with space", "sp ace.txt"]);
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[0].size, 5);
        assert_eq!(entries[0].path, rp("/a.txt"));
        assert!(entries[1].is_dir());

        let st = fs.stat(&rp("/a.txt")).await.unwrap();
        assert_eq!((st.kind, st.size), (EntryKind::File, 5));
        assert!(st.modified.is_some());
        assert!(fs.stat(&rp("/dir with space")).await.unwrap().is_dir());
        assert!(fs.stat(&RemotePath::root()).await.unwrap().is_dir());
        assert!(matches!(
            fs.stat(&rp("/nope")).await,
            Err(Error::NotFound(_))
        ));
        let r = fs.list(&rp("/nope")).await;
        assert!(
            matches!(r, Err(Error::NotFound(_))),
            "{r:?} {}",
            flavor.mlsd
        );

        // write / read round trip, large enough to need many packets
        let data = pattern(300_000);
        put(&fs, "/dir with space/big.bin", 0, &data).await;
        assert_eq!(
            fs.stat(&rp("/dir with space/big.bin")).await.unwrap().size,
            300_000
        );
        assert_eq!(get(&fs, "/dir with space/big.bin", 0).await, data);
        assert_eq!(
            get(&fs, "/dir with space/big.bin", 299_990).await,
            data[299_990..]
        );

        // resume: append at EOF (APPE) and overwrite from the middle (REST+STOR)
        put(&fs, "/dir with space/big.bin", 300_000, b"tail").await;
        assert_eq!(
            get(&fs, "/dir with space/big.bin", 299_998).await,
            [data[299_998], data[299_999], b't', b'a', b'i', b'l']
        );
        put(&fs, "/a.txt", 2, b"LLO!").await;
        assert_eq!(get(&fs, "/a.txt", 0).await, b"heLLO!");
        assert!(matches!(
            fs.open_write(&rp("/a.txt"), 100).await,
            Err(Error::Protocol(_))
        ));
        // truncating rewrite
        put(&fs, "/a.txt", 0, b"new").await;
        assert_eq!(get(&fs, "/a.txt", 0).await, b"new");

        // errors on read / write
        assert!(matches!(
            fs.open_read(&rp("/missing"), 0).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            fs.open_read(&rp("/missing"), 10).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            fs.open_write(&rp("/no/such/dir/f"), 0).await,
            Err(Error::PermissionDenied(_))
        ));

        // mkdir
        fs.mkdir(&rp("/newdir")).await.unwrap();
        assert!(fs.stat(&rp("/newdir")).await.unwrap().is_dir());
        assert!(matches!(
            fs.mkdir(&rp("/newdir")).await,
            Err(Error::AlreadyExists(_))
        ));
        assert!(matches!(
            fs.mkdir(&rp("/missing/child")).await,
            Err(Error::NotFound(_))
        ));

        // rename, remove
        fs.rename(&rp("/a.txt"), &rp("/newdir/b.txt"))
            .await
            .unwrap();
        assert!(matches!(
            fs.stat(&rp("/a.txt")).await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(get(&fs, "/newdir/b.txt", 0).await, b"new");
        assert!(matches!(
            fs.rename(&rp("/a.txt"), &rp("/c.txt")).await,
            Err(Error::NotFound(_))
        ));
        fs.remove(&rp("/newdir/b.txt")).await.unwrap();
        fs.remove(&rp("/newdir")).await.unwrap();
        assert!(!mock.dir.join("newdir").exists());
        assert!(matches!(
            fs.remove(&rp("/newdir")).await,
            Err(Error::NotFound(_))
        ));
    }
}

#[tokio::test]
async fn abandoned_transfers_leave_connection_usable() {
    let mock = Mock::start(Flavor {
        mlsd: true,
        epsv: true,
    })
    .await;
    let fs = Arc::new(FtpFs::connect(mock.opts()).await.unwrap());
    std::fs::write(mock.dir.join("big.bin"), pattern(8_000_000)).unwrap();

    // Read a little, then drop the reader mid-transfer.
    let mut r = fs.open_read(&rp("/big.bin"), 0).await.unwrap();
    let mut buf = [0u8; 16];
    r.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, pattern(16)[..]);
    drop(r);
    // The next operation must wait for the cleanup and then succeed.
    let entries = fs.list(&RemotePath::root()).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(get(&fs, "/big.bin", 7_999_990).await.len(), 10);

    // Drop a writer without shutdown.
    let mut w = fs.open_write(&rp("/partial.bin"), 0).await.unwrap();
    w.write_all(b"abc").await.unwrap();
    drop(w);
    assert_eq!(fs.stat(&rp("/big.bin")).await.unwrap().size, 8_000_000);
    assert_eq!(get(&fs, "/partial.bin", 0).await, b"abc");

    // Operations issued while a transfer is open are serialised behind it.
    let r = fs.open_read(&rp("/partial.bin"), 0).await.unwrap();
    let fs2 = fs.clone();
    let stat = tokio::spawn(async move { fs2.stat(&rp("/partial.bin")).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!stat.is_finished());
    drop(r);
    assert_eq!(stat.await.unwrap().unwrap().size, 3);
}

#[tokio::test]
async fn bad_password_is_auth_error() {
    let mock = Mock::start(Flavor {
        mlsd: true,
        epsv: true,
    })
    .await;
    let mut o = mock.opts();
    o.password = "wrong".into();
    assert!(matches!(FtpFs::connect(o).await, Err(Error::Auth(_))));
}
