//! Headless tests of the GUI service, using `LocalFs` as the "remote" side
//! and a scripted prompter for all interactive questions.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxfer_config::MemoryStore;
use oxfer_core::LocalFs;
use oxfer_gui::service::{
    AuthDto, ConflictChoice, ConnectEnv, ConnectRequest, CredentialDto, EntryKindDto, EventSink,
    HostKeyChoice, OverwriteDto, ProfileDto, ProfileStore, Prompt, PromptAnswer, ProtocolDto,
    QuickAuth, QuickConnect, ScriptedPrompter, SecretKind, SessionManager, TransferEvent,
    TransferOptionsDto, TransferRequest, LOCAL_SESSION,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

struct Fixture {
    svc: Arc<SessionManager>,
    events: UnboundedReceiver<TransferEvent>,
    remote: String,
    dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    fixture_with_env(ConnectEnv::default())
}

fn fixture_with_env(env: ConnectEnv) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let (tx, events) = unbounded_channel();
    let sink: Arc<dyn EventSink> = Arc::new(move |e: TransferEvent| {
        let _ = tx.send(e);
    });
    let profiles = ProfileStore::new(
        dir.path().join("config.toml"),
        Arc::new(MemoryStore::default()),
    );
    let svc = SessionManager::with_local_home(profiles, env, sink, path(dir.path()));
    let remote = svc
        .add_session(Arc::new(LocalFs), "fake", ProtocolDto::Sftp, "/")
        .id;
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::create_dir(dir.path().join("dst")).unwrap();
    Fixture {
        svc,
        events,
        remote,
        dir,
    }
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl Fixture {
    fn p(&self, rel: &str) -> String {
        path(&self.dir.path().join(rel))
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }

    fn request(&self, sources: &[&str], overwrite: OverwriteDto) -> TransferRequest {
        TransferRequest {
            src_session: LOCAL_SESSION.into(),
            dst_session: self.remote.clone(),
            sources: sources.iter().map(|s| self.p(s)).collect(),
            dest_dir: self.p("dst"),
            options: TransferOptionsDto {
                overwrite,
                ..Default::default()
            },
        }
    }

    /// Collect events until `Done` of transfer `id`.
    async fn wait_done(&mut self, id: u64) -> Vec<TransferEvent> {
        let mut out = Vec::new();
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(20), self.events.recv())
                .await
                .expect("timed out waiting for transfer events")
                .expect("event channel closed");
            let done = matches!(ev, TransferEvent::Done { transfer_id, .. } if transfer_id == id);
            out.push(ev);
            if done {
                return out;
            }
        }
    }

    fn drain(&mut self) -> Vec<TransferEvent> {
        let mut v = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            v.push(e);
        }
        v
    }
}

fn done_counts(events: &[TransferEvent]) -> (usize, usize, usize, bool) {
    match events.last() {
        Some(TransferEvent::Done {
            completed,
            skipped,
            failed,
            cancelled,
            ..
        }) => (*completed, *skipped, *failed, *cancelled),
        other => panic!("last event is not Done: {other:?}"),
    }
}

#[tokio::test]
async fn file_operations() {
    let f = fixture();
    let svc = &f.svc;
    f.write("src/a.txt", "hello");
    f.write("src/sub/deep/b.txt", "x");

    let sessions = svc.sessions();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions.iter().find(|s| s.id == "local").unwrap().home,
        path(f.dir.path())
    );

    let mut names: Vec<_> = svc
        .list_dir(&f.remote, &f.p("src"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.kind))
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        names,
        [
            ("a.txt".to_string(), EntryKindDto::File),
            ("sub".to_string(), EntryKindDto::Dir)
        ]
    );
    let st = svc.stat(&f.remote, &f.p("src/a.txt")).await.unwrap();
    assert_eq!((st.size, st.name.as_str()), (5, "a.txt"));
    assert!(st.modified.unwrap() > 0);

    svc.mkdir(&f.remote, &f.p("src/new")).await.unwrap();
    assert_eq!(
        svc.mkdir(&f.remote, &f.p("src/new"))
            .await
            .unwrap_err()
            .code,
        "alreadyExists"
    );

    svc.rename(&f.remote, &f.p("src/a.txt"), &f.p("src/c.txt"))
        .await
        .unwrap();
    assert_eq!(f.read("src/c.txt"), "hello");
    // never overwrite on rename
    let e = svc
        .rename(&f.remote, &f.p("src/c.txt"), &f.p("src/new"))
        .await
        .unwrap_err();
    assert_eq!(e.code, "alreadyExists");

    svc.delete(&f.remote, &[f.p("src/sub"), f.p("src/c.txt")])
        .await
        .unwrap();
    let left: Vec<_> = svc
        .list_dir(&f.remote, &f.p("src"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(left, ["new"]);
    assert_eq!(
        svc.stat(&f.remote, &f.p("src/nope"))
            .await
            .unwrap_err()
            .code,
        "notFound"
    );
    assert_eq!(
        svc.delete(&f.remote, &["/".into()]).await.unwrap_err().code,
        "invalid"
    );

    // sessions
    assert_eq!(
        svc.list_dir("s999", "/").await.unwrap_err().code,
        "unknownSession"
    );
    assert_eq!(svc.disconnect(LOCAL_SESSION).unwrap_err().code, "invalid");
    svc.disconnect(&f.remote).unwrap();
    assert_eq!(svc.sessions().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_reports_events() {
    let mut f = fixture();
    f.write("src/a.txt", "aaaa");
    f.write("src/dir/b.txt", "bb");
    f.write("src/dir/sub/c.txt", "c");
    let prompter = ScriptedPrompter::new([]);
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/a.txt", "src/dir"], OverwriteDto::Ask),
            prompter.clone(),
        )
        .await
        .unwrap();
    let events = f.wait_done(id).await;
    assert!(matches!(
        &events[0],
        TransferEvent::Queued { transfer_id, sources, retry_of: None, .. }
            if *transfer_id == id && sources.len() == 2
    ));
    assert!(events.iter().any(|e| matches!(
        e,
        TransferEvent::Planned {
            files: 3,
            bytes: 7,
            ..
        }
    )));
    let started = events
        .iter()
        .filter(|e| matches!(e, TransferEvent::Started { .. }))
        .count();
    let finished = events
        .iter()
        .filter(|e| matches!(e, TransferEvent::Finished { .. }))
        .count();
    assert_eq!((started, finished), (3, 3));
    assert!(events.iter().any(|e| matches!(
        e,
        TransferEvent::Progress { done, total, .. } if done == total
    )));
    assert_eq!(done_counts(&events), (3, 0, 0, false));
    assert_eq!(f.read("dst/a.txt"), "aaaa");
    assert_eq!(f.read("dst/dir/sub/c.txt"), "c");
    assert!(prompter.asked().is_empty(), "no conflicts expected");
    assert!(f.svc.is_transfer_done(id).unwrap());

    // JSON shape for the UI
    let v = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(v["type"], "queued");
    assert!(v.get("transferId").is_some() && v.get("destDir").is_some());

    // nothing left to retry
    assert_eq!(
        f.svc
            .retry_transfer(id, prompter.clone())
            .await
            .unwrap_err()
            .code,
        "invalid"
    );
    f.svc.clear_finished();
    assert_eq!(
        f.svc.pause_transfer(id).unwrap_err().code,
        "unknownTransfer"
    );
}

#[tokio::test]
async fn pause_and_resume() {
    // current-thread runtime: the transfer task cannot run before we pause it
    let mut f = fixture();
    f.write("src/a.txt", "data");
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/a.txt"], OverwriteDto::Overwrite),
            ScriptedPrompter::new([]),
        )
        .await
        .unwrap();
    f.svc.pause_transfer(id).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let so_far = f.drain();
    assert!(so_far
        .iter()
        .any(|e| matches!(e, TransferEvent::Paused { .. })));
    assert!(
        !so_far.iter().any(|e| matches!(
            e,
            TransferEvent::Finished { .. } | TransferEvent::Done { .. }
        )),
        "{so_far:?}"
    );
    assert!(!f.dir.path().join("dst/a.txt").exists());
    assert!(!f.svc.is_transfer_done(id).unwrap());

    f.svc.resume_transfer(id).unwrap();
    let events = f.wait_done(id).await;
    assert!(matches!(events[0], TransferEvent::Resumed { .. }));
    assert_eq!(done_counts(&events), (1, 0, 0, false));
    assert_eq!(f.read("dst/a.txt"), "data");
}

#[tokio::test]
async fn cancel_then_retry() {
    let mut f = fixture();
    f.write("src/d/a.txt", "1");
    f.write("src/d/b.txt", "22");
    let prompter = ScriptedPrompter::new([]);
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/d"], OverwriteDto::Overwrite),
            prompter.clone(),
        )
        .await
        .unwrap();
    f.svc.pause_transfer(id).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.svc.cancel_transfer(id).unwrap();
    let events = f.wait_done(id).await;
    let (completed, _, failed, cancelled) = done_counts(&events);
    assert_eq!((completed, failed, cancelled), (0, 0, true));
    assert!(!f.dir.path().join("dst/d/a.txt").exists());

    let retry = f.svc.retry_transfer(id, prompter).await.unwrap();
    assert_ne!(retry, id);
    let events = f.wait_done(retry).await;
    assert!(events.iter().any(|e| matches!(
        e,
        TransferEvent::Queued { retry_of: Some(r), .. } if *r == id
    )));
    assert_eq!(done_counts(&events), (2, 0, 0, false));
    assert_eq!(f.read("dst/d/b.txt"), "22");
}

#[tokio::test(flavor = "multi_thread")]
async fn planning_failure_and_validation() {
    let mut f = fixture();
    let prompter = ScriptedPrompter::new([]);
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/missing.txt"], OverwriteDto::Overwrite),
            prompter.clone(),
        )
        .await
        .unwrap();
    let events = f.wait_done(id).await;
    assert!(events.iter().any(|e| matches!(
        e,
        TransferEvent::Failed { job_id: None, error, .. } if error.contains("not found")
    )));
    assert_eq!(done_counts(&events), (0, 0, 1, false));

    // retry re-plans; now the file exists
    f.write("src/missing.txt", "here");
    let retry = f.svc.retry_transfer(id, prompter.clone()).await.unwrap();
    let events = f.wait_done(retry).await;
    assert_eq!(done_counts(&events), (1, 0, 0, false));

    // same directory / into itself / empty / unknown session
    let mut same = f.request(&["src/missing.txt"], OverwriteDto::Overwrite);
    same.dst_session = LOCAL_SESSION.into();
    same.dest_dir = f.p("src");
    assert_eq!(
        f.svc
            .start_transfer(same.clone(), prompter.clone())
            .await
            .unwrap_err()
            .code,
        "invalid"
    );
    same.sources = vec![f.p("src")];
    same.dest_dir = f.p("src/inner");
    assert_eq!(
        f.svc
            .start_transfer(same.clone(), prompter.clone())
            .await
            .unwrap_err()
            .code,
        "invalid"
    );
    same.sources.clear();
    assert!(f
        .svc
        .start_transfer(same.clone(), prompter.clone())
        .await
        .is_err());
    let mut bad = f.request(&["src/missing.txt"], OverwriteDto::Overwrite);
    bad.dst_session = "nope".into();
    assert_eq!(
        f.svc.start_transfer(bad, prompter).await.unwrap_err().code,
        "unknownSession"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn conflicts_are_asked_with_apply_to_all() {
    let mut f = fixture();
    for n in ["a", "b", "c"] {
        f.write(&format!("src/{n}.txt"), "new");
        f.write(&format!("dst/{n}.txt"), "old");
    }
    let prompter = ScriptedPrompter::new([
        PromptAnswer::Conflict {
            action: ConflictChoice::Overwrite,
            apply_to_all: false,
        },
        PromptAnswer::Conflict {
            action: ConflictChoice::Skip,
            apply_to_all: true,
        },
    ]);
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/a.txt", "src/b.txt", "src/c.txt"], OverwriteDto::Ask),
            prompter.clone(),
        )
        .await
        .unwrap();
    let events = f.wait_done(id).await;
    let asked = prompter.asked();
    assert_eq!(asked.len(), 2, "apply-to-all must stop further questions");
    assert!(matches!(
        &asked[0],
        Prompt::Conflict { transfer_id, src, dst }
            if *transfer_id == id && src.size == 3 && dst.size == 3
    ));
    assert_eq!(done_counts(&events), (1, 2, 0, false));
    let skipped: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            TransferEvent::Skipped { path, reason, .. } => Some((path.clone(), reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(skipped.len(), 2);
    assert!(skipped
        .iter()
        .all(|(p, r)| p.ends_with(".txt") && r == "skipped by user"));
    let contents: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|n| f.read(&format!("dst/{n}.txt")))
        .collect();
    assert_eq!(contents.iter().filter(|c| *c == "new").count(), 1);

    // policy "skip" never asks
    let prompter = ScriptedPrompter::new([]);
    let id = f
        .svc
        .start_transfer(
            f.request(&["src/a.txt"], OverwriteDto::Skip),
            prompter.clone(),
        )
        .await
        .unwrap();
    assert_eq!(done_counts(&f.wait_done(id).await), (0, 1, 0, false));
    assert!(prompter.asked().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn interactive_connect_questions() {
    let f = fixture();
    // SFTP with password auth and no password: asks; dismissing cancels.
    let prompter = ScriptedPrompter::new([PromptAnswer::Cancel]);
    let req = ConnectRequest::Quick(QuickConnect {
        protocol: ProtocolDto::Sftp,
        host: "127.0.0.1".into(),
        port: Some(1),
        user: Some("bob".into()),
        auth: QuickAuth::Password,
        password: None,
        key_path: None,
        remote_dir: None,
        insecure_tls: false,
        credential: None,
    });
    let e = f.svc.connect(req, prompter.clone()).await.unwrap_err();
    assert_eq!(e.code, "cancelled");
    assert!(matches!(
        &prompter.asked()[..],
        [Prompt::Password { user, secret: SecretKind::Password, retry: false, .. }] if user == "bob"
    ));

    // Plain FTP must be confirmed first.
    let prompter = ScriptedPrompter::new([PromptAnswer::Confirm { ok: false }]);
    let req: ConnectRequest = serde_json::from_value(serde_json::json!({
        "kind": "quick",
        "protocol": "ftp",
        "host": "127.0.0.1",
        "port": 1,
        "user": "u",
        "auth": "password",
        "password": "p"
    }))
    .unwrap();
    let e = f.svc.connect(req, prompter.clone()).await.unwrap_err();
    assert_eq!(e.code, "cancelled");
    assert!(matches!(
        &prompter.asked()[..],
        [Prompt::Confirm { danger: true, .. }]
    ));

    // unknown profile
    let e = f
        .svc
        .connect(
            ConnectRequest::Profile {
                name: "nope".into(),
            },
            ScriptedPrompter::new([]),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "notFound");
}

macro_rules! server {
    ($tag:expr) => {
        match oxfer_testutil::Server::start($tag) {
            Some(s) => s,
            None => {
                eprintln!("skipping: sshd not available");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn sftp_end_to_end_with_host_key_prompt() {
    let srv = server!("gui");
    let empty_cfg = srv.dir.path().join("ssh_config");
    std::fs::write(&empty_cfg, "").unwrap();
    let mut f = fixture_with_env(ConnectEnv {
        ssh_config: Some(empty_cfg),
        known_hosts: Some(srv.known_hosts()),
        use_default_identities: false,
    });
    let work = path(&srv.work());
    let quick = QuickConnect {
        protocol: ProtocolDto::Sftp,
        host: "127.0.0.1".into(),
        port: Some(srv.port),
        user: Some(srv.user()),
        auth: QuickAuth::KeyFile,
        password: None,
        key_path: Some(path(&srv.client_key())),
        remote_dir: Some(work.clone()),
        insecure_tls: false,
        credential: None,
    };

    // Reject: no connection, nothing saved.
    let prompter = ScriptedPrompter::new([PromptAnswer::HostKey {
        decision: HostKeyChoice::Reject,
    }]);
    let e = f
        .svc
        .connect(ConnectRequest::Quick(quick.clone()), prompter)
        .await
        .unwrap_err();
    assert_eq!(e.code, "hostKey");

    // Accept and save.
    let prompter = ScriptedPrompter::new([PromptAnswer::HostKey {
        decision: HostKeyChoice::AcceptAndSave,
    }]);
    let session = f
        .svc
        .connect(ConnectRequest::Quick(quick.clone()), prompter.clone())
        .await
        .unwrap();
    assert!(matches!(
        &prompter.asked()[..],
        [Prompt::HostKey { fingerprint, port, .. }]
            if fingerprint.starts_with("SHA256:") && *port == srv.port
    ));
    assert_eq!(session.protocol, ProtocolDto::Sftp);
    assert!(session.home.ends_with("work"), "{}", session.home);

    // Upload through the transfer engine and list remotely.
    f.write("src/up.txt", "uploaded");
    let id = f
        .svc
        .start_transfer(
            TransferRequest {
                src_session: LOCAL_SESSION.into(),
                dst_session: session.id.clone(),
                sources: vec![f.p("src/up.txt")],
                dest_dir: session.home.clone(),
                options: TransferOptionsDto::default(),
            },
            prompter.clone(),
        )
        .await
        .unwrap();
    assert_eq!(done_counts(&f.wait_done(id).await), (1, 0, 0, false));
    let names: Vec<_> = f
        .svc
        .list_dir(&session.id, &session.home)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["up.txt"]);

    // A saved profile connects without asking again (key is known now).
    f.svc
        .profiles()
        .save(
            None,
            ProfileDto {
                name: "test".into(),
                protocol: ProtocolDto::Sftp,
                host: "127.0.0.1".into(),
                port: Some(srv.port),
                user: Some(srv.user()),
                auth: AuthDto::KeyFile {
                    path: path(&srv.client_key()),
                },
                remote_dir: Some(work),
                group: None,
                credential: None,
            },
            None,
        )
        .unwrap();
    let prompter = ScriptedPrompter::new([]);
    let s2 = f
        .svc
        .connect(
            ConnectRequest::Profile {
                name: "test".into(),
            },
            prompter.clone(),
        )
        .await
        .unwrap();
    assert!(prompter.asked().is_empty());
    assert_eq!(s2.label, "test");
    f.svc.disconnect(&s2.id).unwrap();

    // A shared credential works for a profile and for a quick connect.
    f.svc
        .profiles()
        .save_credential(
            None,
            CredentialDto {
                name: "testkey".into(),
                user: Some(srv.user()),
                auth: AuthDto::KeyFile {
                    path: path(&srv.client_key()),
                },
            },
            None,
        )
        .unwrap();
    f.svc
        .profiles()
        .save(
            None,
            ProfileDto {
                name: "viacred".into(),
                protocol: ProtocolDto::Sftp,
                host: "127.0.0.1".into(),
                port: Some(srv.port),
                user: None,
                auth: AuthDto::Agent,
                remote_dir: None,
                group: None,
                credential: Some("testkey".into()),
            },
            None,
        )
        .unwrap();
    let s3 = f
        .svc
        .connect(
            ConnectRequest::Profile {
                name: "viacred".into(),
            },
            prompter.clone(),
        )
        .await
        .unwrap();
    f.svc.disconnect(&s3.id).unwrap();
    let s4 = f
        .svc
        .connect(
            ConnectRequest::Quick(QuickConnect {
                protocol: ProtocolDto::Sftp,
                host: "127.0.0.1".into(),
                port: Some(srv.port),
                user: Some("nobody".into()),
                auth: QuickAuth::Password,
                password: None,
                key_path: None,
                remote_dir: None,
                insecure_tls: false,
                credential: Some("testkey".into()),
            }),
            prompter.clone(),
        )
        .await
        .unwrap();
    assert_eq!(s4.label, format!("{}@127.0.0.1", srv.user()));
    assert!(prompter.asked().is_empty());
    f.svc.disconnect(&s4.id).unwrap();
    f.svc.disconnect(&session.id).unwrap();
}
