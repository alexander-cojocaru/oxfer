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
    /// A remote screen (VNC / macOS Screen Sharing), not a file transfer.
    Vnc,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::FtpsExplicit => 21,
            Protocol::FtpsImplicit => 990,
            Protocol::Vnc => 5900,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::FtpsExplicit => "ftps-explicit",
            Protocol::FtpsImplicit => "ftps-implicit",
            Protocol::Vnc => "vnc",
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

/// See [`Profile::vnc_secret_key`].
pub fn vnc_secret_key(profile: &str) -> String {
    format!("{profile}\u{1f}vnc")
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
    /// Folder the profile is listed under; `None` lists it at the top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Name of a shared [`Credential`](crate::Credential). When set, its user
    /// and authentication replace this profile's (see
    /// [`Config::resolve`](crate::Config::resolve)); `auth` is then ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
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
            group: None,
            credential: None,
        }
    }

    /// Key of this profile's password in the [`SecretStore`]: the shared
    /// credential's if it uses one, else its own.
    pub fn secret_key(&self) -> String {
        match &self.credential {
            Some(c) => crate::credential::secret_key(c),
            None => self.name.clone(),
        }
    }

    /// Key of this profile's own VNC password in the [`SecretStore`]. Profile
    /// names cannot contain control characters, so it never collides with a
    /// profile's or credential's key.
    pub fn vnc_secret_key(&self) -> String {
        vnc_secret_key(&self.name)
    }

    /// Case-insensitive search: every whitespace-separated term must occur in
    /// the name, host, user, group or remote directory.
    pub fn matches(&self, query: &str) -> bool {
        let fields = [
            Some(self.name.as_str()),
            Some(self.host.as_str()),
            self.user.as_deref(),
            self.group.as_deref(),
            self.remote_dir.as_deref(),
        ];
        let haystack = fields
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| haystack.contains(term))
    }

    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Invalid(format!("'{}': {m}", self.name)));
        if self.name.trim().is_empty()
            || self.name.chars().any(char::is_control)
            || self.name.starts_with(crate::CREDENTIAL_SECRET_PREFIX)
        {
            return Err(Error::Invalid(format!("bad profile name {:?}", self.name)));
        }
        if self.host.trim().is_empty() {
            return bad("host is empty");
        }
        if self.port == Some(0) {
            return bad("port must not be 0");
        }
        if let Some(g) = &self.group {
            // `/` separates sub groups (`Customers/ACME`); no empty or padded levels.
            let clean = g.split('/').all(|s| !s.is_empty() && s.trim() == s);
            if !clean || g.chars().any(char::is_control) {
                return bad("bad group name");
            }
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
                    .get(&self.secret_key())?
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
    fn search_matches_every_term_in_any_field() {
        let mut p = prof(AuthMethod::Agent);
        p.host = "web.example.com".into();
        p.group = Some("Customers/ACME".into());
        p.remote_dir = Some("/srv/www".into());
        for q in ["", "WEB", "acme bob", "srv customers", "  example  "] {
            assert!(p.matches(q), "{q:?}");
        }
        for q in ["db", "acme alice"] {
            assert!(!p.matches(q), "{q:?}");
        }
    }

    #[test]
    fn group_must_be_a_clean_name() {
        let mut p = prof(AuthMethod::Agent);
        for good in ["Work", "Customers/ACME", "a/b/c", "a b"] {
            p.group = Some(good.into());
            assert!(p.validate().is_ok(), "{good:?}");
        }
        for bad in ["", " ", " Work", "a\tb", "/a", "a/", "a//b", "a/ b/c"] {
            p.group = Some(bad.into());
            assert!(p.validate().is_err(), "{bad:?}");
        }
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
