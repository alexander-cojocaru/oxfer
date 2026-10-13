mod connect;
mod ops;
mod profile;
mod progress;
mod remote;
mod shell;
mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use connect::{connect, ConnArgs};
use oxfer_core::{LocalFs, RemotePath};
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
    /// Print shell completions to stdout
    Completions { shell: clap_complete::Shell },
    /// Print the man page (roff) to stdout
    #[command(hide = true)]
    Man,
    /// Manage saved connection profiles (use them as @name)
    Profile {
        #[command(subcommand)]
        action: profile::Action,
    },
    /// Manage credentials that several profiles can share
    #[command(alias = "cred")]
    Credential {
        #[command(subcommand)]
        action: profile::CredentialAction,
    },
    /// Interactive shell on a remote host
    #[command(alias = "connect")]
    Shell { remote: RemoteSpec },
    /// Dual-pane terminal file manager
    Tui { remote: RemoteSpec },
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
    Mkdir {
        remote: RemoteSpec,
        /// Create missing parent directories (no error if it already exists)
        #[arg(short, long)]
        parents: bool,
    },
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
    RemotePath::from_local(p)
}

async fn run(cli: Cli) -> Result<bool> {
    let conn = &cli.conn;
    match &cli.command {
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), "oxfer", &mut std::io::stdout());
        }
        Command::Man => {
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
        }
        Command::Profile { action } => profile::run(action)?,
        Command::Credential { action } => profile::run_credential(action)?,
        Command::Shell { remote } => {
            let c = connect(conn, remote).await?;
            shell::run(c.fs, c.label, c.path.as_str()).await?;
        }
        Command::Tui { remote } => {
            let c = connect(conn, remote).await?;
            tui::run(c.fs, c.label, c.path.as_str()).await?;
        }
        Command::Ls { remote } => {
            let c = connect(conn, remote).await?;
            let mut entries = c.fs.list(&c.path).await?;
            entries.sort_by(|a, b| a.name().cmp(b.name()));
            for e in &entries {
                println!("{}", ops::format_entry(e));
            }
        }
        Command::Stat { remote } => {
            let c = connect(conn, remote).await?;
            println!("{}", ops::format_entry(&c.fs.stat(&c.path).await?));
        }
        Command::Get {
            remote,
            local: dest,
            transfer,
        } => {
            let c = connect(conn, remote).await?;
            let dest = dest.clone().unwrap_or_else(|| PathBuf::from("."));
            let dest = local(&dest.to_string_lossy());
            let s = ops::copy(c.fs, Arc::new(LocalFs), &[c.path], &dest, transfer).await?;
            return Ok(s.is_success());
        }
        Command::Put { paths, transfer } => {
            let (dest, sources) = paths.split_last().context("missing destination")?;
            let dest: RemoteSpec = dest.parse()?;
            let c = connect(conn, &dest).await?;
            let sources: Vec<_> = sources.iter().map(|s| local(s)).collect();
            let s = ops::copy(Arc::new(LocalFs), c.fs, &sources, &c.path, transfer).await?;
            return Ok(s.is_success());
        }
        Command::Mkdir { remote, parents } => {
            let c = connect(conn, remote).await?;
            if *parents {
                oxfer_core::mkdir_all(&*c.fs, &c.path).await?;
            } else {
                c.fs.mkdir(&c.path).await?;
            }
        }
        Command::Rm { remote, recursive } => {
            let c = connect(conn, remote).await?;
            if *recursive {
                oxfer_core::remove_all(&*c.fs, &c.path).await?;
            } else {
                c.fs.remove(&c.path).await?;
            }
        }
        Command::Mv { from, to } => {
            if from.host != to.host || from.user != to.user || from.profile != to.profile {
                bail!("mv works within a single host");
            }
            let c = connect(conn, from).await?;
            c.fs.rename(&c.path, &rpath(to)).await?;
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
