use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxfer_core::transfer::{
    ConflictAction, Control, Event, OverwritePolicy, Transfer, TransferOptions, PART_SUFFIX,
};
use oxfer_core::{BoxRead, BoxWrite, Entry, Error, LocalFs, RemoteFs, RemotePath, Result};
use tempfile::TempDir;

fn rp(p: &Path) -> RemotePath {
    RemotePath::from_local(&p.to_string_lossy())
}

fn opts() -> TransferOptions {
    TransferOptions {
        recursive: true,
        retry_delay: Duration::from_millis(1),
        ..Default::default()
    }
}

fn transfer(o: TransferOptions) -> Transfer {
    Transfer::new(Arc::new(LocalFs), Arc::new(LocalFs), o)
}

fn write(p: &Path, content: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

struct Dirs {
    _t: TempDir,
    src: std::path::PathBuf,
    dst: std::path::PathBuf,
}

fn dirs() -> Dirs {
    let t = TempDir::new().unwrap();
    let src = t.path().join("src");
    let dst = t.path().join("dst");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    Dirs { _t: t, src, dst }
}

#[tokio::test]
async fn copies_directory_tree() {
    let d = dirs();
    write(&d.src.join("tree/a.txt"), b"aaa");
    write(&d.src.join("tree/sub/b.txt"), &vec![7u8; 1_000_000]);
    std::fs::create_dir_all(d.src.join("tree/empty")).unwrap();

    let s = transfer(opts())
        .copy(
            &[rp(&d.src.join("tree"))],
            &rp(&d.dst),
            None,
            Control::new(),
        )
        .await
        .unwrap();
    assert!(s.is_success(), "{s:?}");
    assert_eq!(s.completed, 2);
    assert_eq!(std::fs::read(d.dst.join("tree/a.txt")).unwrap(), b"aaa");
    assert_eq!(
        std::fs::read(d.dst.join("tree/sub/b.txt")).unwrap().len(),
        1_000_000
    );
    assert!(d.dst.join("tree/empty").is_dir());
    assert!(!d.dst.join(format!("tree/a.txt{PART_SUFFIX}")).exists());
}

#[tokio::test]
async fn directory_requires_recursive() {
    let d = dirs();
    write(&d.src.join("tree/a.txt"), b"x");
    let t = transfer(TransferOptions {
        recursive: false,
        ..opts()
    });
    let err = t
        .copy(
            &[rp(&d.src.join("tree"))],
            &rp(&d.dst),
            None,
            Control::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn glob_expansion() {
    let d = dirs();
    write(&d.src.join("a.csv"), b"1");
    write(&d.src.join("b.csv"), b"2");
    write(&d.src.join("c.txt"), b"3");
    let s = transfer(opts())
        .copy(
            &[rp(&d.src.join("*.csv"))],
            &rp(&d.dst),
            None,
            Control::new(),
        )
        .await
        .unwrap();
    assert_eq!(s.completed, 2);
    assert!(d.dst.join("a.csv").exists() && d.dst.join("b.csv").exists());
    assert!(!d.dst.join("c.txt").exists());

    let err = transfer(opts())
        .copy(
            &[rp(&d.src.join("*.nope"))],
            &rp(&d.dst),
            None,
            Control::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[tokio::test]
async fn glob_can_be_disabled_for_literal_names() {
    let d = dirs();
    write(&d.src.join("[1].txt"), b"literal");
    write(&d.src.join("1.txt"), b"other");
    let t = transfer(TransferOptions {
        glob: false,
        ..opts()
    });
    let s = t
        .copy(
            &[rp(&d.src.join("[1].txt"))],
            &rp(&d.dst),
            None,
            Control::new(),
        )
        .await
        .unwrap();
    assert_eq!(s.completed, 1);
    assert_eq!(std::fs::read(d.dst.join("[1].txt")).unwrap(), b"literal");
    assert!(!d.dst.join("1.txt").exists());
}

#[tokio::test]
async fn overwrite_policies() {
    let d = dirs();
    write(&d.src.join("f"), b"new");
    write(&d.dst.join("f"), b"old");
    let src = [rp(&d.src.join("f"))];

    let o = TransferOptions {
        overwrite: OverwritePolicy::Skip,
        ..opts()
    };
    let s = transfer(o)
        .copy(&src, &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!((s.completed, s.skipped.len()), (0, 1));
    assert_eq!(std::fs::read(d.dst.join("f")).unwrap(), b"old");

    // destination is newer than (or as new as) the source -> keep it
    let o = TransferOptions {
        overwrite: OverwritePolicy::NewerOnly,
        ..opts()
    };
    let s = transfer(o)
        .copy(&src, &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.skipped.len(), 1);

    // make the source clearly newer
    let past = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(d.dst.join("f"))
        .unwrap()
        .set_modified(past)
        .unwrap();
    let o = TransferOptions {
        overwrite: OverwritePolicy::NewerOnly,
        ..opts()
    };
    let s = transfer(o)
        .copy(&src, &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.completed, 1);
    assert_eq!(std::fs::read(d.dst.join("f")).unwrap(), b"new");

    // Ask: resolver says skip, then overwrite
    write(&d.dst.join("f"), b"old");
    let o = TransferOptions {
        overwrite: OverwritePolicy::Ask,
        on_conflict: Some(Arc::new(|_| Box::pin(async { ConflictAction::Skip }))),
        ..opts()
    };
    let s = transfer(o)
        .copy(&src, &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.skipped.len(), 1);
    let o = TransferOptions {
        overwrite: OverwritePolicy::Ask,
        on_conflict: Some(Arc::new(|_| Box::pin(async { ConflictAction::Overwrite }))),
        ..opts()
    };
    let s = transfer(o)
        .copy(&src, &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.completed, 1);
    assert_eq!(std::fs::read(d.dst.join("f")).unwrap(), b"new");
}

#[tokio::test]
async fn resumes_partial_file_and_verifies() {
    let d = dirs();
    let content: Vec<u8> = (0..600_000u32).map(|i| (i % 251) as u8).collect();
    write(&d.src.join("big"), &content);
    write(
        &d.dst.join(format!("big{PART_SUFFIX}")),
        &content[..250_000],
    );

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let o = TransferOptions {
        verify: true,
        ..opts()
    };
    let s = transfer(o)
        .copy(
            &[rp(&d.src.join("big"))],
            &rp(&d.dst),
            Some(tx),
            Control::new(),
        )
        .await
        .unwrap();
    assert!(s.is_success(), "{s:?}");
    assert_eq!(std::fs::read(d.dst.join("big")).unwrap(), content);

    // the first progress event reports the resumed offset
    let mut first = None;
    while let Ok(ev) = rx.try_recv() {
        if let Event::Progress { done, .. } = ev {
            first.get_or_insert(done);
        }
    }
    assert_eq!(first, Some(250_000));
}

#[tokio::test]
async fn corrupt_partial_is_caught_by_verify() {
    let d = dirs();
    write(&d.src.join("f"), &[1u8; 100_000]);
    write(&d.dst.join(format!("f{PART_SUFFIX}")), &[9u8; 40_000]); // wrong prefix
    let o = TransferOptions {
        verify: true,
        retries: 0,
        ..opts()
    };
    let s = transfer(o)
        .copy(&[rp(&d.src.join("f"))], &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.failed.len(), 1);
    assert!(!d.dst.join("f").exists());
    // the bad partial was discarded, so a second run succeeds
    let o = TransferOptions {
        verify: true,
        retries: 0,
        ..opts()
    };
    let s = transfer(o)
        .copy(&[rp(&d.src.join("f"))], &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert!(s.is_success());
}

/// Fails `open_read` the first `n` times with a transient error.
struct Flaky {
    inner: LocalFs,
    failures_left: AtomicU32,
}

#[async_trait]
impl RemoteFs for Flaky {
    async fn list(&self, p: &RemotePath) -> Result<Vec<Entry>> {
        self.inner.list(p).await
    }
    async fn stat(&self, p: &RemotePath) -> Result<Entry> {
        self.inner.stat(p).await
    }
    async fn open_read(&self, p: &RemotePath, o: u64) -> Result<BoxRead> {
        if self
            .failures_left
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(Error::Connection("flaky".into()));
        }
        self.inner.open_read(p, o).await
    }
    async fn open_write(&self, p: &RemotePath, o: u64) -> Result<BoxWrite> {
        self.inner.open_write(p, o).await
    }
    async fn mkdir(&self, p: &RemotePath) -> Result<()> {
        self.inner.mkdir(p).await
    }
    async fn remove(&self, p: &RemotePath) -> Result<()> {
        self.inner.remove(p).await
    }
    async fn rename(&self, a: &RemotePath, b: &RemotePath) -> Result<()> {
        self.inner.rename(a, b).await
    }
}

fn flaky(n: u32) -> Arc<Flaky> {
    Arc::new(Flaky {
        inner: LocalFs,
        failures_left: AtomicU32::new(n),
    })
}

#[tokio::test]
async fn retries_transient_errors() {
    let d = dirs();
    write(&d.src.join("f"), b"data");
    let t = Transfer::new(
        flaky(2),
        Arc::new(LocalFs),
        TransferOptions {
            retries: 3,
            ..opts()
        },
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let s = t
        .copy(
            &[rp(&d.src.join("f"))],
            &rp(&d.dst),
            Some(tx),
            Control::new(),
        )
        .await
        .unwrap();
    assert!(s.is_success(), "{s:?}");
    let mut retries = 0;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, Event::Retrying { .. }) {
            retries += 1;
        }
    }
    assert_eq!(retries, 2);

    let d = dirs();
    write(&d.src.join("f"), b"data");
    let t = Transfer::new(
        flaky(1),
        Arc::new(LocalFs),
        TransferOptions {
            retries: 0,
            ..opts()
        },
    );
    let s = t
        .copy(&[rp(&d.src.join("f"))], &rp(&d.dst), None, Control::new())
        .await
        .unwrap();
    assert_eq!(s.failed.len(), 1);
}

#[tokio::test]
async fn cancelled_before_start_copies_nothing() {
    let d = dirs();
    write(&d.src.join("f"), b"data");
    let c = Control::new();
    c.cancel();
    let s = transfer(opts())
        .copy(&[rp(&d.src.join("f"))], &rp(&d.dst), None, c)
        .await
        .unwrap();
    assert!(s.cancelled && !s.is_success());
    assert!(!d.dst.join("f").exists());
}

#[tokio::test]
async fn pause_blocks_until_resumed() {
    let d = dirs();
    write(&d.src.join("f"), b"data");
    let c = Control::new();
    c.pause();
    let t = transfer(opts());
    let (src, dst) = (rp(&d.src.join("f")), rp(&d.dst));
    let c2 = c.clone();
    let h = tokio::spawn(async move { t.copy(&[src], &dst, None, c2).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!d.dst.join("f").exists(), "must not progress while paused");
    c.resume();
    let s = tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .unwrap()
        .unwrap();
    assert!(s.is_success());
}

/// Lists hostile paths that try to escape the destination.
struct Hostile;

#[async_trait]
impl RemoteFs for Hostile {
    async fn list(&self, p: &RemotePath) -> Result<Vec<Entry>> {
        let mk = |path: &str| Entry {
            path: RemotePath::new(path),
            kind: oxfer_core::EntryKind::File,
            size: 1,
            modified: None,
            permissions: None,
        };
        Ok(vec![mk(&format!("{p}/../../evil")), mk(&format!("{p}/ok"))])
    }
    async fn stat(&self, p: &RemotePath) -> Result<Entry> {
        Ok(Entry {
            path: p.clone(),
            kind: if p.as_str() == "/root" {
                oxfer_core::EntryKind::Dir
            } else {
                oxfer_core::EntryKind::File
            },
            size: 1,
            modified: None,
            permissions: None,
        })
    }
    async fn open_read(&self, _: &RemotePath, _: u64) -> Result<BoxRead> {
        Ok(Box::new(&b"x"[..]))
    }
    async fn open_write(&self, _: &RemotePath, _: u64) -> Result<BoxWrite> {
        Err(Error::Unsupported("write"))
    }
    async fn mkdir(&self, _: &RemotePath) -> Result<()> {
        Err(Error::Unsupported("mkdir"))
    }
    async fn remove(&self, _: &RemotePath) -> Result<()> {
        Err(Error::Unsupported("remove"))
    }
    async fn rename(&self, _: &RemotePath, _: &RemotePath) -> Result<()> {
        Err(Error::Unsupported("rename"))
    }
}

#[tokio::test]
async fn path_traversal_from_server_is_skipped() {
    let d = dirs();
    let t = Transfer::new(Arc::new(Hostile), Arc::new(LocalFs), opts());
    let plan = t
        .plan(&[RemotePath::new("/root")], &rp(&d.dst))
        .await
        .unwrap();
    assert_eq!(plan.jobs.len(), 1);
    assert_eq!(plan.jobs[0].dst, rp(&d.dst).join("root").join("ok"));
    assert_eq!(plan.skipped.len(), 1);
    assert!(plan.skipped[0].1.contains("unsafe"));
}
