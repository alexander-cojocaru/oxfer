//! Integration tests against a minimal in-process mock FTP server (no external
//! tools needed, so they are deterministic and never skipped). The mock serves
//! a temp directory and is run in several flavours: with/without MLSD/MLST and
//! with/without EPSV, to exercise the fallbacks.

mod common;

use std::sync::Arc;

use common::*;
use oxfer_core::{EntryKind, Error, RemoteFs, RemotePath};
use oxfer_ftp::FtpFs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
