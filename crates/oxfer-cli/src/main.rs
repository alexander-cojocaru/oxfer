mod remote;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use oxfer_core::{Entry, RemoteFs, RemotePath};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, HostKeyPrompt, SftpFs};
use remote::RemoteSpec;
use tokio::io::AsyncWriteExt;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "oxfer", version, about = "SFTP/FTP client")]
struct Cli {
    /// SSH port
    #[arg(short = 'P', long, default_value_t = 22, global = true)]
    port: u16,
    /// Private key file
    #[arg(short = 'i', long, global = true)]
    identity: Option<PathBuf>,
    /// Authenticate with the ssh-agent only
    #[arg(long, global = true)]
    agent: bool,
    /// Use this known_hosts file instead of ~/.ssh/known_hosts
    #[arg(long, global = true)]
    known_hosts: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List a remote directory
    Ls { remote: RemoteSpec },
    /// Show metadata of a remote path
    Stat { remote: RemoteSpec },
    /// Download a file
    Get {
        remote: RemoteSpec,
        local: Option<PathBuf>,
    },
    /// Upload a file
    Put { local: PathBuf, remote: RemoteSpec },
    /// Create a remote directory
    Mkdir { remote: RemoteSpec },
    /// Remove a remote file or empty directory
    Rm { remote: RemoteSpec },
    /// Rename a path on the same host
    Mv { from: RemoteSpec, to: RemoteSpec },
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn confirm_host_key(p: &HostKeyPrompt) -> HostKeyDecision {
    eprintln!(
        "The authenticity of host '{}:{}' can't be established.\n{} key fingerprint is {}.",
        p.host, p.port, p.algorithm, p.fingerprint
    );
    eprint!("Trust this host and save it to known_hosts? [yes/no/once] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return HostKeyDecision::Reject;
    }
    match line.trim() {
        "yes" => HostKeyDecision::AcceptAndSave,
        "once" => HostKeyDecision::AcceptOnce,
        _ => HostKeyDecision::Reject,
    }
}

fn default_key() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| Path::new(&home).join(".ssh").join(n))
        .find(|p| p.exists())
}

async fn connect(cli: &Cli, spec: &RemoteSpec) -> Result<SftpFs> {
    let user = spec
        .user
        .clone()
        .or_else(|| std::env::var("USER").ok())
        .context("no user given (use user@host)")?;

    let auth = if let Some(path) = &cli.identity {
        Auth::Key {
            path: path.clone(),
            passphrase: None,
        }
    } else if cli.agent || std::env::var_os("SSH_AUTH_SOCK").is_some() {
        Auth::Agent
    } else {
        Auth::Password(rpassword::prompt_password(format!(
            "{user}@{}'s password: ",
            spec.host
        ))?)
    };

    let mut opts = ConnectOptions::new(spec.host.clone(), user.clone(), auth);
    opts.port = cli.port;
    opts.known_hosts = cli.known_hosts.clone();
    opts.verifier = Arc::new(confirm_host_key);

    match SftpFs::connect(opts.clone()).await {
        Ok(fs) => Ok(fs),
        // Agent failed or had no usable identity: fall back to a key file or password.
        Err(oxfer_core::Error::Auth(_)) if matches!(opts.auth, Auth::Agent) && !cli.agent => {
            opts.auth = match default_key() {
                Some(path) => Auth::Key {
                    path,
                    passphrase: None,
                },
                None => Auth::Password(rpassword::prompt_password(format!(
                    "{user}@{}'s password: ",
                    spec.host
                ))?),
            };
            Ok(SftpFs::connect(opts).await?)
        }
        Err(e) => Err(e.into()),
    }
}

fn rpath(spec: &RemoteSpec) -> RemotePath {
    RemotePath::new(spec.path.clone())
}

fn print_entry(e: &Entry) {
    let kind = match e.kind {
        oxfer_core::EntryKind::Dir => 'd',
        oxfer_core::EntryKind::Symlink => 'l',
        oxfer_core::EntryKind::File => '-',
        oxfer_core::EntryKind::Other => '?',
    };
    let perm = e.permissions.map_or("----".into(), |p| format!("{p:04o}"));
    let mtime = e
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    println!("{kind}{perm} {:>12} {mtime:>11} {}", e.size, e.name());
}

async fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Ls { remote } => {
            let fs = connect(&cli, remote).await?;
            let mut entries = fs.list(&rpath(remote)).await?;
            entries.sort_by(|a, b| a.name().cmp(b.name()));
            entries.iter().for_each(print_entry);
        }
        Command::Stat { remote } => {
            let fs = connect(&cli, remote).await?;
            print_entry(&fs.stat(&rpath(remote)).await?);
        }
        Command::Get { remote, local } => {
            let fs = connect(&cli, remote).await?;
            let path = rpath(remote);
            let name = path.file_name().context("remote path has no file name")?;
            let dest = match local {
                Some(l) if l.is_dir() => l.join(name),
                Some(l) => l.clone(),
                None => PathBuf::from(name),
            };
            let mut src = fs.open_read(&path, 0).await?;
            let mut out = tokio::fs::File::create(&dest).await?;
            let n = tokio::io::copy(&mut src, &mut out).await?;
            out.flush().await?;
            eprintln!("{} -> {} ({n} bytes)", path, dest.display());
        }
        Command::Put { local, remote } => {
            let fs = connect(&cli, remote).await?;
            let mut dest = rpath(remote);
            // `host:` or an existing remote directory means "into that directory".
            let is_dir =
                remote.path == "." || fs.stat(&dest).await.map(|e| e.is_dir()).unwrap_or(false);
            if is_dir {
                let name = local.file_name().context("local path has no file name")?;
                dest = dest.join(&name.to_string_lossy());
            }
            let mut src = tokio::fs::File::open(local).await?;
            let mut out = fs.open_write(&dest, 0).await?;
            let n = tokio::io::copy(&mut src, &mut out).await?;
            out.shutdown().await?;
            eprintln!("{} -> {} ({n} bytes)", local.display(), dest);
        }
        Command::Mkdir { remote } => {
            connect(&cli, remote).await?.mkdir(&rpath(remote)).await?;
        }
        Command::Rm { remote } => {
            connect(&cli, remote).await?.remove(&rpath(remote)).await?;
        }
        Command::Mv { from, to } => {
            if from.host != to.host || from.user != to.user {
                bail!("mv works within a single host");
            }
            connect(&cli, from)
                .await?
                .rename(&rpath(from), &rpath(to))
                .await?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("oxfer: {e}");
        std::process::exit(1);
    }
}
