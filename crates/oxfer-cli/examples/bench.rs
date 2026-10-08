//! Benchmark `oxfer` against OpenSSH `sftp` over loopback to a throwaway sshd.
//!
//!     cargo build --release -p oxfer-cli && cargo run --release -p oxfer-cli --example bench
//!
//! Loopback numbers measure client overhead, not network performance.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use oxfer_testutil::Server;

fn time(mut c: Command) -> Duration {
    let t = Instant::now();
    let s = c
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn");
    assert!(s.success(), "command failed: {c:?}");
    t.elapsed()
}

fn main() {
    let oxfer = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/oxfer");
    assert!(oxfer.exists(), "build with --release first");
    let srv = Server::start("bench").expect("sshd not available");
    let work = srv.work();
    let local = std::env::temp_dir().join(format!("oxfer-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(local.join("many")).unwrap();

    let big: Vec<u8> = (0..256 * 1024 * 1024u64).map(|i| (i % 253) as u8).collect();
    std::fs::write(local.join("big.bin"), &big).unwrap();
    drop(big);
    for i in 0..1000 {
        std::fs::write(local.join(format!("many/f{i:04}.txt")), vec![b'x'; 4096]).unwrap();
    }

    let user = srv.user();
    let port = srv.port.to_string();
    let key = srv.client_key();
    let kh = srv.known_hosts();
    // record the host key once
    let _ = Command::new("ssh-keyscan")
        .args(["-p", &port, "127.0.0.1"])
        .stdout(std::fs::File::create(&kh).unwrap())
        .stderr(Stdio::null())
        .status();

    let remote = |sub: &str| format!("{user}@127.0.0.1:{}/{sub}", work.display());
    let oxfer_cmd = |args: &[&str]| {
        let mut c = Command::new(&oxfer);
        c.args(["-P", &port, "-i"])
            .arg(&key)
            .args(["--ssh-config", "/nonexistent", "--known-hosts"])
            .arg(&kh)
            .args(args);
        c
    };
    let sftp_batch = |batch: &str| {
        let b = local.join("batch.txt");
        std::fs::write(&b, batch).unwrap();
        let mut c = Command::new("sftp");
        c.args(["-q", "-P", &port, "-i"])
            .arg(&key)
            .args(["-o", "StrictHostKeyChecking=no", "-o", "IdentitiesOnly=yes"])
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", kh.display()))
            .arg("-b")
            .arg(&b)
            .arg(format!("{user}@127.0.0.1"));
        c
    };

    std::fs::create_dir_all(work.join("o")).unwrap();
    std::fs::create_dir_all(work.join("s")).unwrap();
    let big = local.join("big.bin").display().to_string();
    let many = local.join("many").display().to_string();
    let dl = |n: &str| {
        let d = local.join(n);
        std::fs::create_dir_all(&d).unwrap();
        d.display().to_string()
    };

    println!("{:<34} {:>10} {:>10}", "scenario", "oxfer", "sftp");
    let row = |name: &str, a: Duration, b: Duration| {
        println!("{name:<34} {:>9.2}s {:>9.2}s", a.as_secs_f64(), b.as_secs_f64())
    };

    let a = time(oxfer_cmd(&["put", &big, &remote("o")]));
    let b = time(sftp_batch(&format!("put {big} {}/s/big.bin\n", work.display())));
    row("upload 256 MiB file", a, b);

    let (d1, d2) = (dl("dl1"), dl("dl2"));
    let a = time(oxfer_cmd(&["get", &remote("o/big.bin"), &d1]));
    let b = time(sftp_batch(&format!("get {}/s/big.bin {d2}/big.bin\n", work.display())));
    row("download 256 MiB file", a, b);

    let a = time(oxfer_cmd(&["put", "-r", &many, &remote("o")]));
    let b = time(sftp_batch(&format!("put -r {many} {}/s\n", work.display())));
    row("upload 1000 x 4 KiB files", a, b);

    let (d3, d4) = (dl("dl3"), dl("dl4"));
    let a = time(oxfer_cmd(&["get", "-r", &remote("o/many"), &d3]));
    let b = time(sftp_batch(&format!("get -r {}/s/many {d4}\n", work.display())));
    row("download 1000 x 4 KiB files", a, b);

    let _ = std::fs::remove_dir_all(&local);
}
