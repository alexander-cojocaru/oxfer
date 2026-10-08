use crate::secrets::SecretStore;
use crate::ssh_config::{expand_tilde, SshConfig};
use crate::{Error, Result};
use oxfer_sftp::{Auth, ConnectOptions};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    Sftp,
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::FtpsExplicit => 21,
            Protocol::FtpsImplicit => 990,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::FtpsExplicit => "ftps-explicit",
            Protocol::FtpsImplicit => "ftps-implicit",
        }
    }
}

/// How to authenticate. Passwords live in the keychain, never in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum AuthMethod {
    Agent,
    KeyFile { path: PathBuf },
    Password,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub auth: AuthMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_dir: Option<String>,
    /// Profile name or `user@host:port`. Not supported for connecting yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
}

impl Profile {
    pub fn new(
        name: impl Into<String>,
        protocol: Protocol,
        host: impl Into<String>,
        auth: AuthMethod,
    ) -> Self {
        Self {
            name: name.into(),
            protocol,
            host: host.into(),
            port: None,
            user: None,
            auth,
            remote_dir: None,
            proxy_jump: None,
        }
    }

    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Invalid(format!("'{}': {m}", self.name)));
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            return Err(Error::Invalid(format!("bad profile name {:?}", self.name)));
        }
        if self.host.trim().is_empty() {
            return bad("host is empty");
        }
        if self.port == Some(0) {
            return bad("port must not be 0");
        }
        Ok(())
    }

    /// Build SFTP connect options, merging profile values over `ssh_config`
    /// defaults (the profile host is treated as a possible `Host` alias).
    pub fn to_sftp_options(
        &self,
        secrets: &dyn SecretStore,
        ssh: Option<&SshConfig>,
    ) -> Result<ConnectOptions> {
        if self.protocol != Protocol::Sftp {
            return Err(Error::WrongProtocol(self.protocol.name()));
        }
        let resolved = ssh.map(|s| s.resolve(&self.host));
        let proxy = self
            .proxy_jump
            .clone()
            .or_else(|| resolved.as_ref().and_then(|r| r.proxy_jump.clone()));
        let host = resolved
            .as_ref()
            .map_or_else(|| self.host.clone(), |r| r.hostname.clone());
        let user = self
            .user
            .clone()
            .or_else(|| resolved.as_ref().and_then(|r| r.user.clone()))
            .ok_or_else(|| Error::MissingUser(self.name.clone()))?;
        let port = self
            .port
            .or_else(|| resolved.as_ref().and_then(|r| r.port))
            .unwrap_or(22);
        let auth = match &self.auth {
            AuthMethod::Agent => Auth::Agent,
            AuthMethod::KeyFile { path } => Auth::Key {
                path: expand_tilde(path, ssh.and_then(|s| s.home())),
                passphrase: None,
            },
            AuthMethod::Password => {
                let pw = secrets
                    .get(&self.name)?
                    .ok_or_else(|| Error::MissingSecret(self.name.clone()))?;
                Auth::Password(pw.expose().to_string())
            }
            AuthMethod::Interactive => return Err(Error::InteractiveAuth(self.name.clone())),
        };
        let mut opts = ConnectOptions::new(host, user, auth);
        opts.port = port;
        if let Some(p) = proxy {
            opts.jump = crate::jump_chain(&p, ssh)?;
        }
        Ok(opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, Secret};

    fn prof(auth: AuthMethod) -> Profile {
        let mut p = Profile::new("web", Protocol::Sftp, "web", auth);
        p.user = Some("bob".into());
        p
    }

    #[test]
    fn defaults_without_ssh_config() {
        let o = prof(AuthMethod::Agent)
            .to_sftp_options(&MemoryStore::default(), None)
            .unwrap();
        assert_eq!(
            (o.host.as_str(), o.port, o.user.as_str()),
            ("web", 22, "bob")
        );
        assert!(matches!(o.auth, Auth::Agent));
    }

    #[test]
    fn profile_overrides_ssh_config() {
        let cfg = SshConfig::parse("Host web\n HostName 10.0.0.1\n Port 2222\n User alice\n");
        let mut p = prof(AuthMethod::Agent);
        let store = MemoryStore::default();
        let o = p.to_sftp_options(&store, Some(&cfg)).unwrap();
        assert_eq!(
            (o.host.as_str(), o.port, o.user.as_str()),
            ("10.0.0.1", 2222, "bob")
        );
        p.port = Some(22);
        p.user = None;
        let o = p.to_sftp_options(&store, Some(&cfg)).unwrap();
        assert_eq!((o.port, o.user.as_str()), (22, "alice"));
    }

    #[test]
    fn password_from_store() {
        let store = MemoryStore::default();
        let p = prof(AuthMethod::Password);
        assert!(matches!(
            p.to_sftp_options(&store, None),
            Err(Error::MissingSecret(_))
        ));
        store.set("web", &Secret::new("pw")).unwrap();
        let o = p.to_sftp_options(&store, None).unwrap();
        assert!(matches!(o.auth, Auth::Password(ref s) if s == "pw"));
    }

    #[test]
    fn interactive_proxy_protocol_errors() {
        let store = MemoryStore::default();
        assert!(matches!(
            prof(AuthMethod::Interactive).to_sftp_options(&store, None),
            Err(Error::InteractiveAuth(_))
        ));
        let mut p = prof(AuthMethod::Agent);
        p.proxy_jump = Some("ops@jump:2200".into());
        let o = p.to_sftp_options(&store, None).unwrap();
        let j = o.jump.expect("jump configured");
        assert_eq!(
            (j.host.as_str(), j.port, j.user.as_str()),
            ("jump", 2200, "ops")
        );
        let cfg = SshConfig::parse("Host web\n ProxyJump bastion\n");
        let o = prof(AuthMethod::Agent)
            .to_sftp_options(&store, Some(&cfg))
            .unwrap();
        assert_eq!(o.jump.unwrap().host, "bastion");
        let mut p = prof(AuthMethod::Agent);
        p.protocol = Protocol::Ftp;
        assert!(matches!(
            p.to_sftp_options(&store, None),
            Err(Error::WrongProtocol("ftp"))
        ));
        p.protocol = Protocol::Sftp;
        p.user = None;
        assert!(matches!(
            p.to_sftp_options(&store, None),
            Err(Error::MissingUser(_))
        ));
    }

    #[test]
    fn key_file_expands_tilde() {
        let mut cfg = SshConfig::parse("");
        cfg.set_home("/home/u");
        let p = prof(AuthMethod::KeyFile {
            path: "~/.ssh/id".into(),
        });
        let o = p
            .to_sftp_options(&MemoryStore::default(), Some(&cfg))
            .unwrap();
        assert!(
            matches!(o.auth, Auth::Key { ref path, .. } if path == &PathBuf::from("/home/u/.ssh/id"))
        );
    }
}
