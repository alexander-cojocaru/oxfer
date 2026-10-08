use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use oxfer_config::{AuthMethod, Config, KeyringStore, Profile, Protocol, SecretStore, SshConfig};
use oxfer_core::{RemoteFs, RemotePath};
use oxfer_ftp::{FtpConnectOptions, FtpFs, TlsMode};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, HostKeyPrompt, SftpFs};

use crate::remote::RemoteSpec;

/// Connection settings shared by every command.
#[derive(clap::Args, Clone, Debug)]
pub struct ConnArgs {
    /// SSH port
    #[arg(short = 'P', long, global = true)]
    pub port: Option<u16>,
    /// Private key file
    #[arg(short = 'i', long, global = true)]
    pub identity: Option<PathBuf>,
    /// Authenticate with the ssh-agent only
    #[arg(long, global = true)]
    pub agent: bool,
    /// Use this known_hosts file instead of ~/.ssh/known_hosts
    #[arg(long, global = true)]
    pub known_hosts: Option<PathBuf>,
    /// Use this ssh config instead of ~/.ssh/config
    #[arg(long, global = true)]
    pub ssh_config: Option<PathBuf>,
    /// Accept invalid TLS certificates for FTPS (insecure)
    #[arg(long, global = true)]
    pub insecure_tls: bool,
}

/// An open connection plus where to start.
pub struct Conn {
    pub fs: Arc<dyn RemoteFs>,
    pub path: RemotePath,
    pub label: String,
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

fn default_keys() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| Path::new(&home).join(".ssh").join(n))
        .filter(|p| p.exists())
        .collect()
}

pub fn ask_password(user: &str, host: &str) -> Result<String> {
    Ok(rpassword::prompt_password(format!(
        "{user}@{host}'s password: "
    ))?)
}

fn load_ssh_config(args: &ConnArgs) -> SshConfig {
    let path = args.ssh_config.clone().or_else(SshConfig::default_path);
    path.and_then(|p| SshConfig::load(p).ok())
        .unwrap_or_else(|| SshConfig::parse(""))
}

pub async fn connect(args: &ConnArgs, spec: &RemoteSpec) -> Result<Conn> {
    let mut path = spec.path.clone();
    let (fs, label): (Arc<dyn RemoteFs>, String) = if let Some(name) = &spec.profile {
        let config = Config::load().context("cannot read profiles")?;
        let profile = config
            .get(name)
            .with_context(|| format!("no profile named {name:?} (see `oxfer profile list`)"))?
            .clone();
        if !spec.has_path() {
            if let Some(dir) = &profile.remote_dir {
                path = dir.clone();
            }
        }
        let fs = connect_profile(args, &profile).await?;
        (fs, format!("@{name}"))
    } else {
        match spec.protocol {
            Protocol::Sftp => (
                Arc::new(connect_sftp_spec(args, spec).await?),
                format!("{}@{}", spec.user.as_deref().unwrap_or("-"), spec.host),
            ),
            proto => {
                let password = match &spec.user {
                    Some(u) => ask_password(u, &spec.host)?,
                    None => "anonymous@oxfer".into(),
                };
                let user = spec.user.clone().unwrap_or_else(|| "anonymous".into());
                let fs = connect_ftp(
                    args,
                    &spec.host,
                    spec.port.or(args.port),
                    &user,
                    &password,
                    proto,
                )
                .await?;
                (Arc::new(fs), format!("{user}@{}", spec.host))
            }
        }
    };
    Ok(Conn {
        fs,
        path: RemotePath::new(path),
        label,
    })
}

async fn connect_ftp(
    args: &ConnArgs,
    host: &str,
    port: Option<u16>,
    user: &str,
    password: &str,
    protocol: Protocol,
) -> Result<FtpFs> {
    let mut o = FtpConnectOptions::new(host, user, password);
    o.port = port.unwrap_or_else(|| protocol.default_port());
    o.tls = match protocol {
        Protocol::Ftp => TlsMode::None,
        Protocol::FtpsExplicit => TlsMode::Explicit,
        Protocol::FtpsImplicit => TlsMode::Implicit,
        Protocol::Sftp => bail!("not an FTP protocol"),
    };
    o.danger_accept_invalid_certs = args.insecure_tls;
    Ok(FtpFs::connect(o).await?)
}

async fn connect_profile(args: &ConnArgs, profile: &Profile) -> Result<Arc<dyn RemoteFs>> {
    let secrets = KeyringStore;
    let stored_password = || -> Result<Option<String>> {
        Ok(secrets.get(&profile.name)?.map(|s| s.expose().to_string()))
    };
    match profile.protocol {
        Protocol::Sftp => {
            let ssh = load_ssh_config(args);
            let mut opts = match profile.to_sftp_options(&secrets, Some(&ssh)) {
                Ok(o) => o,
                Err(oxfer_config::Error::InteractiveAuth(_))
                | Err(oxfer_config::Error::MissingSecret(_)) => {
                    // Build with an agent placeholder, then ask for the password.
                    let mut p = profile.clone();
                    p.auth = AuthMethod::Agent;
                    let mut o = p.to_sftp_options(&secrets, Some(&ssh))?;
                    o.auth = Auth::Password(ask_password(&o.user, &o.host)?);
                    o
                }
                Err(e) => return Err(e.into()),
            };
            apply_sftp_common(args, &mut opts);
            Ok(Arc::new(SftpFs::connect(opts).await?))
        }
        proto => {
            let user = profile.user.clone().unwrap_or_else(|| "anonymous".into());
            let password = match &profile.auth {
                AuthMethod::Password => match stored_password()? {
                    Some(p) => p,
                    None => ask_password(&user, &profile.host)?,
                },
                AuthMethod::Interactive => ask_password(&user, &profile.host)?,
                _ if profile.user.is_none() => "anonymous@oxfer".into(),
                _ => bail!("FTP profiles need a password or interactive auth"),
            };
            let fs = connect_ftp(
                args,
                &profile.host,
                Some(profile.effective_port()),
                &user,
                &password,
                proto,
            )
            .await?;
            Ok(Arc::new(fs))
        }
    }
}

fn apply_sftp_common(args: &ConnArgs, opts: &mut ConnectOptions) {
    opts.known_hosts = args.known_hosts.clone();
    opts.verifier = Arc::new(confirm_host_key);
    if let Some(jump) = opts.jump.as_mut() {
        apply_sftp_common(args, jump);
    }
}

async fn connect_sftp_spec(args: &ConnArgs, spec: &RemoteSpec) -> Result<SftpFs> {
    let ssh = load_ssh_config(args);
    let resolved = ssh.resolve(&spec.host);
    let user = spec
        .user
        .clone()
        .or(resolved.user.clone())
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .context("no user given (use user@host)")?;
    let host = resolved.hostname.clone();
    let port = spec.port.or(args.port).or(resolved.port).unwrap_or(22);

    let mut keys: Vec<PathBuf> = resolved
        .identity_files
        .iter()
        .filter(|p| p.exists())
        .cloned()
        .collect();
    if !resolved.identities_only {
        let extra: Vec<_> = default_keys()
            .into_iter()
            .filter(|k| !keys.contains(k))
            .collect();
        keys.extend(extra);
    }

    // Authentication order: explicit key / agent flag, else agent, key files, password.
    let mut candidates: Vec<Auth> = Vec::new();
    if let Some(path) = &args.identity {
        candidates.push(Auth::Key {
            path: path.clone(),
            passphrase: None,
        });
    } else if args.agent {
        candidates.push(Auth::Agent);
    } else {
        if std::env::var_os("SSH_AUTH_SOCK").is_some_and(|v| !v.is_empty()) {
            candidates.push(Auth::Agent);
        }
        candidates.extend(keys.into_iter().map(|path| Auth::Key {
            path,
            passphrase: None,
        }));
    }

    let jump = match &resolved.proxy_jump {
        Some(p) => oxfer_config::jump_chain(p, Some(&ssh))?,
        None => None,
    };
    let mut last_err = None;
    for auth in candidates {
        let mut opts = ConnectOptions::new(host.clone(), user.clone(), auth);
        opts.port = port;
        opts.jump = jump.clone();
        apply_sftp_common(args, &mut opts);
        match SftpFs::connect(opts).await {
            Ok(fs) => return Ok(fs),
            Err(e @ oxfer_core::Error::Auth(_)) => last_err = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    if args.identity.is_some() || args.agent {
        return Err(last_err
            .map(Into::into)
            .unwrap_or_else(|| anyhow::anyhow!("authentication failed")));
    }
    let mut opts = ConnectOptions::new(
        host.clone(),
        user.clone(),
        Auth::Password(ask_password(&user, &host)?),
    );
    opts.port = port;
    opts.jump = jump;
    apply_sftp_common(args, &mut opts);
    Ok(SftpFs::connect(opts).await?)
}
