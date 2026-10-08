use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, HostKeyPrompt, SftpFs};

use crate::remote::RemoteSpec;

/// Connection settings shared by every command.
#[derive(clap::Args, Clone, Debug)]
pub struct ConnArgs {
    /// SSH port
    #[arg(short = 'P', long, default_value_t = 22, global = true)]
    pub port: u16,
    /// Private key file
    #[arg(short = 'i', long, global = true)]
    pub identity: Option<PathBuf>,
    /// Authenticate with the ssh-agent only
    #[arg(long, global = true)]
    pub agent: bool,
    /// Use this known_hosts file instead of ~/.ssh/known_hosts
    #[arg(long, global = true)]
    pub known_hosts: Option<PathBuf>,
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

fn ask_password(user: &str, host: &str) -> Result<Auth> {
    Ok(Auth::Password(rpassword::prompt_password(format!(
        "{user}@{host}'s password: "
    ))?))
}

pub async fn connect(args: &ConnArgs, spec: &RemoteSpec) -> Result<SftpFs> {
    let user = spec
        .user
        .clone()
        .or_else(|| std::env::var("USER").ok())
        .context("no user given (use user@host)")?;

    let auth = if let Some(path) = &args.identity {
        Auth::Key {
            path: path.clone(),
            passphrase: None,
        }
    } else if args.agent || std::env::var_os("SSH_AUTH_SOCK").is_some() {
        Auth::Agent
    } else if let Some(path) = default_key() {
        Auth::Key {
            path,
            passphrase: None,
        }
    } else {
        ask_password(&user, &spec.host)?
    };

    let mut opts = ConnectOptions::new(spec.host.clone(), user.clone(), auth);
    opts.port = args.port;
    opts.known_hosts = args.known_hosts.clone();
    opts.verifier = Arc::new(confirm_host_key);

    let mut result = SftpFs::connect(opts.clone()).await;
    // Fall back through key file -> password when the automatic choice failed.
    if matches!(result, Err(oxfer_core::Error::Auth(_))) && args.identity.is_none() && !args.agent {
        if matches!(opts.auth, Auth::Agent) {
            if let Some(path) = default_key() {
                opts.auth = Auth::Key {
                    path,
                    passphrase: None,
                };
                result = SftpFs::connect(opts.clone()).await;
            }
        }
        if matches!(result, Err(oxfer_core::Error::Auth(_))) {
            opts.auth = ask_password(&user, &spec.host)?;
            result = SftpFs::connect(opts).await;
        }
    }
    Ok(result?)
}
