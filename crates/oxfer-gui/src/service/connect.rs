//! Turning a profile or quick-connect parameters into an `Arc<dyn RemoteFs>`.
//! Mirrors `oxfer-cli`'s connect logic, but every question goes through a
//! [`Prompter`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oxfer_config::{AuthMethod, Profile, Protocol, SshConfig};
use oxfer_core::{Error as CoreError, RemoteFs, RemotePath};
use oxfer_ftp::{FtpConnectOptions, FtpFs, TlsMode};
use oxfer_sftp::{Auth, ConnectOptions, HostKeyDecision, HostKeyPrompt, HostKeyVerifier, SftpFs};
use serde::Deserialize;

use super::dto::ProtocolDto;
use super::error::{ServiceError, ServiceResult};
use super::profiles::ProfileStore;
use super::prompt::{
    ask_blocking, HostKeyChoice, Prompt, PromptAnswer, Prompter, SecretKind, SecretString,
};

const MAX_PASSWORD_ATTEMPTS: usize = 3;

/// Environment for outgoing connections (overridable for tests).
#[derive(Debug, Clone, Default)]
pub struct ConnectEnv {
    /// `ssh_config` file; `None` = `~/.ssh/config`.
    pub ssh_config: Option<PathBuf>,
    /// `known_hosts` file; `None` = `~/.ssh/known_hosts`.
    pub known_hosts: Option<PathBuf>,
    /// Try the ssh-agent and `~/.ssh/id_*` keys for quick connects with `auto` auth.
    pub use_default_identities: bool,
}

impl ConnectEnv {
    fn ssh_config(&self) -> SshConfig {
        self.ssh_config
            .clone()
            .or_else(SshConfig::default_path)
            .and_then(|p| SshConfig::load(p).ok())
            .unwrap_or_else(|| SshConfig::parse(""))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuickAuth {
    /// ssh-agent, then ssh_config/default keys, then ask for a password.
    Auto,
    Agent,
    KeyFile,
    Password,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickConnect {
    pub protocol: ProtocolDto,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    pub auth: QuickAuth,
    /// Used once, never stored. Asked for when missing.
    #[serde(default)]
    pub password: Option<SecretString>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub remote_dir: Option<String>,
    /// Accept invalid FTPS certificates without asking (insecure).
    #[serde(default)]
    pub insecure_tls: bool,
    /// Saved credential to use instead of `user`/`auth`/`password`/`key_path`.
    #[serde(default)]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConnectRequest {
    Profile { name: String },
    Quick(QuickConnect),
}

pub(crate) struct Connected {
    pub fs: Arc<dyn RemoteFs>,
    pub label: String,
    pub protocol: ProtocolDto,
    pub start_dir: Option<String>,
    /// Identifies the server and account actually connected to (after
    /// ssh_config resolution); see [`server_key`].
    pub key: String,
}

/// One open session per protocol, account, host and port.
pub(crate) fn server_key(protocol: Protocol, user: &str, host: &str, port: u16) -> String {
    format!(
        "{protocol:?}://{user}@{}:{port}",
        host.trim().to_lowercase()
    )
}

/// The [`server_key`] a request would connect to, as far as it can be told
/// without connecting (ssh_config aliases are only resolved by connecting).
pub(crate) fn request_key(profiles: &ProfileStore, req: &ConnectRequest) -> Option<String> {
    match req {
        ConnectRequest::Profile { name } => {
            let p = profiles.get(name).ok()?;
            let user = p.user.as_deref().unwrap_or_default();
            Some(server_key(p.protocol, user, &p.host, p.effective_port()))
        }
        ConnectRequest::Quick(q) => {
            let protocol = q.protocol.to_config()?;
            let user = match q.credential.as_deref().filter(|c| !c.trim().is_empty()) {
                Some(c) => profiles.credential(c).ok()?.user,
                None => q.user.clone(),
            };
            let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
            let user = match (user, protocol) {
                (Some(u), _) => u,
                (None, Protocol::Sftp) => String::new(),
                (None, _) => "anonymous".into(),
            };
            let port = q.port.unwrap_or_else(|| protocol.default_port());
            Some(server_key(protocol, &user, &q.host, port))
        }
    }
}

fn host_key_verifier(prompter: Arc<dyn Prompter>) -> HostKeyVerifier {
    // Remember the answer so that several authentication attempts within one
    // connect only ask once.
    let memo: Mutex<Option<(String, HostKeyDecision)>> = Mutex::new(None);
    Arc::new(move |p: &HostKeyPrompt| {
        let mut memo = memo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((fp, d)) = memo.as_ref() {
            if *fp == p.fingerprint {
                return *d;
            }
        }
        let answer = ask_blocking(
            &prompter,
            Prompt::HostKey {
                host: p.host.clone(),
                port: p.port,
                algorithm: p.algorithm.clone(),
                fingerprint: p.fingerprint.clone(),
            },
        );
        let d = match answer {
            PromptAnswer::HostKey {
                decision: HostKeyChoice::AcceptAndSave,
            } => HostKeyDecision::AcceptAndSave,
            PromptAnswer::HostKey {
                decision: HostKeyChoice::AcceptOnce,
            } => HostKeyDecision::AcceptOnce,
            _ => HostKeyDecision::Reject,
        };
        *memo = Some((p.fingerprint.clone(), d));
        d
    })
}

async fn ask_secret(
    prompter: &dyn Prompter,
    user: &str,
    host: &str,
    secret: SecretKind,
    retry: bool,
) -> ServiceResult<String> {
    match prompter
        .ask(Prompt::Password {
            user: user.into(),
            host: host.into(),
            secret,
            retry,
        })
        .await
    {
        PromptAnswer::Password { value } => Ok(value.0),
        _ => Err(ServiceError::cancelled()),
    }
}

async fn confirm(prompter: &dyn Prompter, title: &str, message: String) -> ServiceResult<()> {
    match prompter
        .ask(Prompt::Confirm {
            title: title.into(),
            message,
            danger: true,
        })
        .await
    {
        PromptAnswer::Confirm { ok: true } => Ok(()),
        _ => Err(ServiceError::cancelled()),
    }
}

fn default_keys() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return vec![];
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| Path::new(&home).join(".ssh").join(n))
        .filter(|p| p.exists())
        .collect()
}

struct SftpTarget<'a> {
    env: &'a ConnectEnv,
    host: String,
    port: u16,
    user: String,
    verifier: HostKeyVerifier,
    /// ProxyJump chain (from the profile or ssh_config).
    jump: Option<Box<ConnectOptions>>,
}

/// Apply known_hosts and the interactive verifier to every hop of a jump chain.
fn prepare_jump(
    mut jump: Option<Box<ConnectOptions>>,
    env: &ConnectEnv,
    verifier: &HostKeyVerifier,
) -> Option<Box<ConnectOptions>> {
    let mut hop = jump.as_deref_mut();
    while let Some(h) = hop {
        h.known_hosts = env.known_hosts.clone();
        h.verifier = verifier.clone();
        hop = h.jump.as_deref_mut();
    }
    jump
}

impl SftpTarget<'_> {
    async fn try_auth(&self, auth: Auth) -> oxfer_core::Result<SftpFs> {
        let mut o = ConnectOptions::new(self.host.clone(), self.user.clone(), auth);
        o.port = self.port;
        o.known_hosts = self.env.known_hosts.clone();
        o.verifier = self.verifier.clone();
        o.jump = self.jump.clone();
        SftpFs::connect(o).await
    }

    /// Try `candidates` in order; `explicit_key` candidates may ask for a
    /// passphrase; finally ask for a password if `password_fallback`.
    async fn connect(
        &self,
        candidates: Vec<Auth>,
        explicit_key: bool,
        password_fallback: bool,
        prompter: &dyn Prompter,
    ) -> ServiceResult<SftpFs> {
        let mut last_err: Option<CoreError> = None;
        for auth in candidates {
            match self.try_auth(auth.clone()).await {
                Ok(fs) => return Ok(fs),
                Err(CoreError::Auth(m)) if explicit_key && m.starts_with("cannot load key") => {
                    let Auth::Key { path, .. } = auth else {
                        unreachable!("only key auth loads keys")
                    };
                    for attempt in 0..MAX_PASSWORD_ATTEMPTS {
                        let pass = ask_secret(
                            prompter,
                            &path.to_string_lossy(),
                            &self.host,
                            SecretKind::Passphrase,
                            attempt > 0,
                        )
                        .await?;
                        match self
                            .try_auth(Auth::Key {
                                path: path.clone(),
                                passphrase: Some(pass),
                            })
                            .await
                        {
                            Ok(fs) => return Ok(fs),
                            Err(e @ CoreError::Auth(_)) => last_err = Some(e),
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                Err(e @ CoreError::Auth(_)) => last_err = Some(e),
                Err(e) => return Err(e.into()),
            }
        }
        if password_fallback {
            for attempt in 0..MAX_PASSWORD_ATTEMPTS {
                let pw = ask_secret(
                    prompter,
                    &self.user,
                    &self.host,
                    SecretKind::Password,
                    attempt > 0,
                )
                .await?;
                match self.try_auth(Auth::Password(pw)).await {
                    Ok(fs) => return Ok(fs),
                    Err(e @ CoreError::Auth(_)) => last_err = Some(e),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Err(last_err
            .map(Into::into)
            .unwrap_or_else(|| ServiceError::new("auth", "no authentication method available")))
    }
}

/// Plain FTP: confirm first. FTPS: offer to accept an invalid certificate.
async fn connect_ftp(
    protocol: Protocol,
    host: &str,
    port: u16,
    user: &str,
    password: Option<String>,
    insecure_tls: bool,
    prompter: &dyn Prompter,
) -> ServiceResult<FtpFs> {
    let tls = match protocol {
        Protocol::Ftp => TlsMode::None,
        Protocol::FtpsExplicit => TlsMode::Explicit,
        Protocol::FtpsImplicit => TlsMode::Implicit,
        Protocol::Sftp => return Err(ServiceError::invalid("not an FTP protocol")),
    };
    if tls == TlsMode::None {
        confirm(
            prompter,
            "Unencrypted connection",
            format!(
                "Plain FTP sends your password and files to {host} unencrypted. Connect anyway?"
            ),
        )
        .await?;
    }
    let mut insecure = insecure_tls;
    let mut password = password;
    let mut prompted = 0;
    loop {
        let pw = match &password {
            Some(p) => p.clone(),
            None => {
                let p =
                    ask_secret(prompter, user, host, SecretKind::Password, prompted > 0).await?;
                prompted += 1;
                p
            }
        };
        let mut o = FtpConnectOptions::new(host, user, pw);
        o.port = port;
        o.tls = tls;
        o.danger_accept_invalid_certs = insecure;
        match FtpFs::connect(o).await {
            Ok(fs) => return Ok(fs),
            Err(e)
                if !insecure
                    && tls != TlsMode::None
                    && e.to_string().to_lowercase().contains("certificate") =>
            {
                confirm(
                    prompter,
                    "Untrusted certificate",
                    format!(
                        "The TLS certificate of {host} could not be verified ({e}). \
                         Someone could be intercepting the connection. Connect anyway?"
                    ),
                )
                .await?;
                insecure = true;
            }
            Err(CoreError::Auth(m)) if prompted < MAX_PASSWORD_ATTEMPTS => {
                tracing::debug!("FTP login failed: {m}");
                password = None;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

pub(crate) async fn connect(
    env: &ConnectEnv,
    profiles: &ProfileStore,
    req: ConnectRequest,
    prompter: Arc<dyn Prompter>,
) -> ServiceResult<Connected> {
    match req {
        ConnectRequest::Profile { name } => {
            let profile = profiles.get(&name)?;
            connect_profile(env, profiles, &profile, prompter).await
        }
        ConnectRequest::Quick(q) => {
            let q = apply_credential(profiles, q)?;
            connect_quick(env, q, prompter).await
        }
    }
}

/// Replace the user and authentication of a quick connect with those of its
/// saved credential.
fn apply_credential(profiles: &ProfileStore, mut q: QuickConnect) -> ServiceResult<QuickConnect> {
    let Some(name) = q.credential.take().filter(|n| !n.trim().is_empty()) else {
        return Ok(q);
    };
    let c = profiles.credential(&name)?;
    q.user = c.user.clone();
    q.password = None;
    q.key_path = None;
    q.auth = match &c.auth {
        AuthMethod::Agent => QuickAuth::Agent,
        AuthMethod::KeyFile { path } => {
            q.key_path = Some(path.to_string_lossy().into_owned());
            QuickAuth::KeyFile
        }
        AuthMethod::Password => {
            q.password = profiles
                .secrets()
                .get(&c.secret_key())?
                .map(|s| SecretString(s.expose().to_string()));
            QuickAuth::Password
        }
        // Asked for while connecting.
        AuthMethod::Interactive => QuickAuth::Password,
    };
    Ok(q)
}

async fn connect_profile(
    env: &ConnectEnv,
    profiles: &ProfileStore,
    profile: &Profile,
    prompter: Arc<dyn Prompter>,
) -> ServiceResult<Connected> {
    let (fs, key): (Arc<dyn RemoteFs>, String) = match profile.protocol {
        Protocol::Sftp => {
            // SshConfig is not Send: keep it out of the awaits below.
            let (opts, password_fallback) = {
                let ssh = env.ssh_config();
                let secrets = profiles.secrets();
                match profile.to_sftp_options(secrets, Some(&ssh)) {
                    Ok(o) => (o, profile.auth == AuthMethod::Password),
                    Err(oxfer_config::Error::InteractiveAuth(_))
                    | Err(oxfer_config::Error::MissingSecret(_)) => {
                        let mut p = profile.clone();
                        p.auth = AuthMethod::Agent;
                        (p.to_sftp_options(secrets, Some(&ssh))?, true)
                    }
                    Err(e) => return Err(e.into()),
                }
            };
            let candidates = match profile.auth {
                AuthMethod::Interactive => vec![],
                AuthMethod::Password if password_fallback && matches!(opts.auth, Auth::Agent) => {
                    vec![]
                }
                _ => vec![opts.auth.clone()],
            };
            let verifier = host_key_verifier(prompter.clone());
            let target = SftpTarget {
                env,
                host: opts.host.clone(),
                port: opts.port,
                user: opts.user.clone(),
                jump: prepare_jump(opts.jump.clone(), env, &verifier),
                verifier,
            };
            let explicit_key = matches!(profile.auth, AuthMethod::KeyFile { .. });
            let fs = target
                .connect(candidates, explicit_key, password_fallback, &*prompter)
                .await?;
            let key = server_key(Protocol::Sftp, &target.user, &target.host, target.port);
            (Arc::new(fs), key)
        }
        proto => {
            let user = profile.user.clone().unwrap_or_else(|| "anonymous".into());
            let password = match &profile.auth {
                AuthMethod::Password => profiles
                    .secrets()
                    .get(&profile.secret_key())?
                    .map(|s| s.expose().to_string()),
                AuthMethod::Interactive => None,
                _ if profile.user.is_none() => Some("anonymous@oxfer".into()),
                _ => {
                    return Err(ServiceError::invalid(
                        "FTP profiles need a password or interactive auth",
                    ))
                }
            };
            let fs = connect_ftp(
                proto,
                &profile.host,
                profile.effective_port(),
                &user,
                password,
                false,
                &*prompter,
            )
            .await?;
            let key = server_key(proto, &user, &profile.host, profile.effective_port());
            (Arc::new(fs), key)
        }
    };
    Ok(Connected {
        fs,
        label: profile.name.clone(),
        protocol: profile.protocol.into(),
        start_dir: profile.remote_dir.clone(),
        key,
    })
}

async fn connect_quick(
    env: &ConnectEnv,
    q: QuickConnect,
    prompter: Arc<dyn Prompter>,
) -> ServiceResult<Connected> {
    let host = q.host.trim().to_string();
    if host.is_empty() {
        return Err(ServiceError::invalid("host is empty"));
    }
    let protocol = q
        .protocol
        .to_config()
        .ok_or_else(|| ServiceError::invalid("cannot connect to the local protocol"))?;
    let user_in = q
        .user
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(String::from);
    let password = q.password.map(|p| p.0).filter(|p| !p.is_empty());
    let (fs, label, key): (Arc<dyn RemoteFs>, String, String) = match protocol {
        Protocol::Sftp => {
            let (resolved, ssh_home, jump) = {
                let ssh = env.ssh_config();
                let resolved = ssh.resolve(&host);
                let jump = match &resolved.proxy_jump {
                    Some(p) => oxfer_config::jump_chain(p, Some(&ssh))?,
                    None => None,
                };
                (resolved, ssh.home().map(Path::to_path_buf), jump)
            };
            let user = user_in
                .or(resolved.user.clone())
                .or_else(|| std::env::var("USER").ok())
                .or_else(|| std::env::var("USERNAME").ok())
                .ok_or_else(|| ServiceError::invalid("no user given"))?;
            let port = q.port.or(resolved.port).unwrap_or(22);
            let mut candidates = Vec::new();
            let mut explicit_key = false;
            let mut password_fallback = false;
            match q.auth {
                QuickAuth::Agent => candidates.push(Auth::Agent),
                QuickAuth::KeyFile => {
                    let path = q
                        .key_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .ok_or_else(|| ServiceError::invalid("no key file given"))?;
                    candidates.push(Auth::Key {
                        path: oxfer_config::ssh_config::expand_tilde(
                            Path::new(path),
                            ssh_home.as_deref(),
                        ),
                        passphrase: None,
                    });
                    explicit_key = true;
                }
                QuickAuth::Password => match password {
                    Some(pw) => candidates.push(Auth::Password(pw)),
                    None => password_fallback = true,
                },
                QuickAuth::Auto => {
                    let mut keys: Vec<PathBuf> = resolved
                        .identity_files
                        .iter()
                        .filter(|p| p.exists())
                        .cloned()
                        .collect();
                    if env.use_default_identities {
                        if std::env::var_os("SSH_AUTH_SOCK").is_some_and(|v| !v.is_empty()) {
                            candidates.push(Auth::Agent);
                        }
                        if !resolved.identities_only {
                            for k in default_keys() {
                                if !keys.contains(&k) {
                                    keys.push(k);
                                }
                            }
                        }
                    }
                    candidates.extend(keys.into_iter().map(|path| Auth::Key {
                        path,
                        passphrase: None,
                    }));
                    if let Some(pw) = password {
                        candidates.push(Auth::Password(pw));
                    }
                    password_fallback = true;
                }
            }
            let verifier = host_key_verifier(prompter.clone());
            let target = SftpTarget {
                env,
                host: resolved.hostname.clone(),
                port,
                user: user.clone(),
                jump: prepare_jump(jump, env, &verifier),
                verifier,
            };
            let fs = target
                .connect(candidates, explicit_key, password_fallback, &*prompter)
                .await?;
            let key = server_key(Protocol::Sftp, &user, &target.host, port);
            (Arc::new(fs), format!("{user}@{host}"), key)
        }
        proto => {
            let (user, password) = match user_in {
                Some(u) => (u, password),
                None => ("anonymous".to_string(), Some("anonymous@oxfer".to_string())),
            };
            let port = q.port.unwrap_or_else(|| proto.default_port());
            let fs = connect_ftp(
                proto,
                &host,
                port,
                &user,
                password,
                q.insecure_tls,
                &*prompter,
            )
            .await?;
            let key = server_key(proto, &user, &host, port);
            (Arc::new(fs), format!("{user}@{host}"), key)
        }
    };
    Ok(Connected {
        fs,
        label,
        protocol: protocol.into(),
        start_dir: q.remote_dir.filter(|d| !d.trim().is_empty()),
        key,
    })
}

/// Where to start browsing a fresh connection.
pub(crate) async fn start_dir(fs: &dyn RemoteFs, configured: Option<String>) -> String {
    if let Some(d) = configured {
        if let Ok(p) = fs.realpath(&RemotePath::new(d.clone())).await {
            return p.to_string();
        }
        return d;
    }
    match fs.realpath(&RemotePath::new(".")).await {
        Ok(p) if p.is_absolute() => p.to_string(),
        _ => "/".into(),
    }
}
