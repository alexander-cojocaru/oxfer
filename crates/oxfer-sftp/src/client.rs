use std::path::PathBuf;
use std::sync::Arc;

use oxfer_core::{Error, Result};
use russh::client::{self, Handle};
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;

use crate::SftpFs;

#[derive(Clone)]
pub enum Auth {
    Password(String),
    /// Private key file with optional passphrase.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    /// Use identities from the running ssh-agent (`SSH_AUTH_SOCK`).
    Agent,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Password(_) => f.write_str("Password(..)"),
            Auth::Key { path, .. } => write!(f, "Key({})", path.display()),
            Auth::Agent => f.write_str("Agent"),
        }
    }
}

/// Information shown to the user when a host is not yet in `known_hosts`.
#[derive(Debug, Clone)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Trust and record the key in `known_hosts`.
    AcceptAndSave,
    /// Trust for this connection only.
    AcceptOnce,
    Reject,
}

/// Callback asked what to do with an unknown host key. A *changed* key is
/// always rejected without asking.
pub type HostKeyVerifier = Arc<dyn Fn(&HostKeyPrompt) -> HostKeyDecision + Send + Sync>;

#[derive(Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// Defaults to rejecting unknown hosts.
    pub verifier: HostKeyVerifier,
    /// Override the `known_hosts` file (defaults to `~/.ssh/known_hosts`).
    pub known_hosts: Option<PathBuf>,
}

impl ConnectOptions {
    pub fn new(host: impl Into<String>, user: impl Into<String>, auth: Auth) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth,
            verifier: Arc::new(|_| HostKeyDecision::Reject),
            known_hosts: None,
        }
    }
}

struct Handler {
    host: String,
    port: u16,
    verifier: HostKeyVerifier,
    known_hosts: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
enum HandlerError {
    #[error(transparent)]
    Ssh(#[from] russh::Error),
    #[error("{0}")]
    HostKey(String),
}

impl client::Handler for Handler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Err(HandlerError::HostKey(
                "host certificates are not supported".into(),
            ));
        };
        let check = match &self.known_hosts {
            Some(p) => keys::known_hosts::check_known_hosts_path(&self.host, self.port, key, p),
            None => keys::check_known_hosts(&self.host, self.port, key),
        };
        match check {
            Ok(true) => Ok(true),
            Err(keys::Error::KeyChanged { line }) => Err(HandlerError::HostKey(format!(
                "HOST KEY CHANGED for {} (known_hosts line {line}); refusing to connect",
                self.host
            ))),
            Err(e) => Err(HandlerError::HostKey(e.to_string())),
            Ok(false) => {
                let prompt = HostKeyPrompt {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: key.algorithm().to_string(),
                    fingerprint: key.fingerprint(Default::default()).to_string(),
                };
                match (self.verifier)(&prompt) {
                    HostKeyDecision::Reject => {
                        Err(HandlerError::HostKey("host key rejected by user".into()))
                    }
                    HostKeyDecision::AcceptOnce => Ok(true),
                    HostKeyDecision::AcceptAndSave => {
                        let r = match &self.known_hosts {
                            Some(p) => keys::known_hosts::learn_known_hosts_path(
                                &self.host, self.port, key, p,
                            ),
                            None => {
                                keys::known_hosts::learn_known_hosts(&self.host, self.port, key)
                            }
                        };
                        if let Err(e) = r {
                            tracing::warn!("could not save host key: {e}");
                        }
                        Ok(true)
                    }
                }
            }
        }
    }
}

fn conn_err(e: impl std::fmt::Display) -> Error {
    Error::Connection(e.to_string())
}

fn map_connect_err(e: HandlerError) -> Error {
    match e {
        HandlerError::HostKey(m) => Error::HostKey(m),
        HandlerError::Ssh(e) => conn_err(e),
    }
}

impl SftpFs {
    pub async fn connect(opts: ConnectOptions) -> Result<Self> {
        let handler = Handler {
            host: opts.host.clone(),
            port: opts.port,
            verifier: opts.verifier.clone(),
            known_hosts: opts.known_hosts.clone(),
        };
        let config = Arc::new(client::Config::default());
        let mut session = client::connect(config, (opts.host.as_str(), opts.port), handler)
            .await
            .map_err(map_connect_err)?;

        authenticate(&mut session, &opts).await?;

        let channel = session.channel_open_session().await.map_err(conn_err)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(conn_err)?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| Error::Protocol(e.to_string()))?;
        Ok(SftpFs::new(sftp, session))
    }
}

async fn authenticate(session: &mut Handle<Handler>, opts: &ConnectOptions) -> Result<()> {
    let ok = match &opts.auth {
        Auth::Password(pw) => session
            .authenticate_password(&opts.user, pw)
            .await
            .map_err(conn_err)?
            .success(),
        Auth::Key { path, passphrase } => {
            let key = keys::load_secret_key(path, passphrase.as_deref())
                .map_err(|e| Error::Auth(format!("cannot load key {}: {e}", path.display())))?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(conn_err)?
                .flatten();
            session
                .authenticate_publickey(&opts.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(conn_err)?
                .success()
        }
        Auth::Agent => authenticate_agent(session, &opts.user).await?,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::Auth("server rejected credentials".into()))
    }
}

#[cfg(unix)]
async fn authenticate_agent(session: &mut Handle<Handler>, user: &str) -> Result<bool> {
    use russh::keys::agent::client::AgentClient;
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|e| Error::Auth(format!("cannot reach ssh-agent: {e}")))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| Error::Auth(format!("ssh-agent: {e}")))?;
    let hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(conn_err)?
        .flatten();
    for id in identities {
        let key = id.public_key().into_owned();
        let res = session
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await;
        if matches!(res, Ok(r) if r.success()) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(unix))]
async fn authenticate_agent(_: &mut Handle<Handler>, _: &str) -> Result<bool> {
    Err(Error::Unsupported("ssh-agent on this platform"))
}
