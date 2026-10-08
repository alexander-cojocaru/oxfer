mod connect;
mod ops;
mod progress;
mod remote;
mod shell;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use connect::{connect, ConnArgs};
use oxfer_core::{LocalFs, RemoteFs, RemotePath};
use remote::RemoteSpec;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "oxfer", version, about = "SFTP/FTP client")]
struct Cli {
    #[command(flatten)]
    conn: ConnArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive shell on a remote host
    #[command(alias = "connect")]
    Shell { remote: RemoteSpec },
    /// List a remote directory
    Ls { remote: RemoteSpec },
    /// Show metadata of a remote path
    Stat { remote: RemoteSpec },
    /// Download files or directories (globs allowed)
    Get {
        remote: RemoteSpec,
        /// Local file or directory (default: current directory)
        local: Option<PathBuf>,
        #[command(flatten)]
        transfer: ops::TransferArgs,
    },
    /// Upload files or directories to a remote path
    Put {
        /// One or more local paths followed by the remote destination
        #[arg(required = true, num_args = 2..)]
        paths: Vec<String>,
        #[command(flatten)]
        transfer: ops::TransferArgs,
    },
    /// Create a remote directory
    Mkdir { remote: RemoteSpec },
    /// Remove a remote file or directory
    Rm {
        remote: RemoteSpec,
        /// Remove directories and their contents
        #[arg(short, long)]
        recursive: bool,
    },
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

fn rpath(spec: &RemoteSpec) -> RemotePath {
    RemotePath::new(spec.path.clone())
}

fn local(p: &str) -> RemotePath {
    RemotePath::new(p)
}

async fn run(cli: Cli) -> Result<bool> {
    let conn = &cli.conn;
    match &cli.command {
        Command::Shell { remote } => {
            let fs: Arc<dyn RemoteFs> = Arc::new(connect(conn, remote).await?);
            let label = format!("{}@{}", remote.user.as_deref().unwrap_or("-"), remote.host);
            shell::run(fs, label, &remote.path).await?;
        }
        Command::Ls { remote } => {
            let fs = connect(conn, remote).await?;
            let mut entries = fs.list(&rpath(remote)).await?;
            entries.sort_by(|a, b| a.name().cmp(b.name()));
            for e in &entries {
                println!("{}", ops::format_entry(e));
            }
        }
        Command::Stat { remote } => {
            let fs = connect(conn, remote).await?;
            println!("{}", ops::format_entry(&fs.stat(&rpath(remote)).await?));
        }
        Command::Get {
            remote,
            local: dest,
            transfer,
        } => {
            let fs: Arc<dyn RemoteFs> = Arc::new(connect(conn, remote).await?);
            let dest = dest.clone().unwrap_or_else(|| PathBuf::from("."));
            let dest = local(&dest.to_string_lossy());
            let s = ops::copy(fs, Arc::new(LocalFs), &[rpath(remote)], &dest, transfer).await?;
            return Ok(s.is_success());
        }
        Command::Put { paths, transfer } => {
            let (dest, sources) = paths.split_last().context("missing destination")?;
            let dest: RemoteSpec = dest.parse()?;
            let fs: Arc<dyn RemoteFs> = Arc::new(connect(conn, &dest).await?);
            let sources: Vec<_> = sources.iter().map(|s| local(s)).collect();
            let s = ops::copy(Arc::new(LocalFs), fs, &sources, &rpath(&dest), transfer).await?;
            return Ok(s.is_success());
        }
        Command::Mkdir { remote } => {
            connect(conn, remote).await?.mkdir(&rpath(remote)).await?;
        }
        Command::Rm { remote, recursive } => {
            let fs = connect(conn, remote).await?;
            let path = rpath(remote);
            if *recursive {
                oxfer_core::remove_all(&fs, &path).await?;
            } else {
                fs.remove(&path).await?;
            }
        }
        Command::Mv { from, to } => {
            if from.host != to.host || from.user != to.user {
                bail!("mv works within a single host");
            }
            connect(conn, from)
                .await?
                .rename(&rpath(from), &rpath(to))
                .await?;
        }
    }
    Ok(true)
}

#[tokio::main]
async fn main() {
    init_tracing();
    match run(Cli::parse()).await {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("oxfer: {e:#}");
            std::process::exit(1);
        }
    }
}
