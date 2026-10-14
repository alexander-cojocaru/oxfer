//! Saved connection profiles (wrapping `oxfer_config`). Passwords go to the
//! injected [`SecretStore`] (the OS keychain in the app), never to the file.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use oxfer_config::{AuthMethod, Config, Credential, Profile, Secret, SecretStore};
use serde::{Deserialize, Serialize};

use super::dto::ProtocolDto;
use super::error::{ServiceError, ServiceResult};
use super::prompt::SecretString;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImportDto {
    pub added: usize,
    pub replaced: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "method",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthDto {
    Agent,
    KeyFile {
        path: String,
    },
    /// Password stored in the keychain.
    Password,
    /// Ask for the password on every connect.
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    pub name: String,
    pub protocol: ProtocolDto,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    pub auth: AuthDto,
    #[serde(default)]
    pub remote_dir: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    /// Shared credential; when set, `user` and `auth` come from it.
    #[serde(default)]
    pub credential: Option<String>,
}

/// A saved credential that several profiles (and quick connects) can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDto {
    pub name: String,
    #[serde(default)]
    pub user: Option<String>,
    pub auth: AuthDto,
}

impl From<&AuthMethod> for AuthDto {
    fn from(a: &AuthMethod) -> Self {
        match a {
            AuthMethod::Agent => AuthDto::Agent,
            AuthMethod::KeyFile { path } => AuthDto::KeyFile {
                path: path.to_string_lossy().into_owned(),
            },
            AuthMethod::Password => AuthDto::Password,
            AuthMethod::Interactive => AuthDto::Interactive,
        }
    }
}

impl AuthDto {
    fn into_method(self) -> ServiceResult<AuthMethod> {
        Ok(match self {
            AuthDto::Agent => AuthMethod::Agent,
            AuthDto::KeyFile { path } if path.trim().is_empty() => {
                return Err(ServiceError::invalid("key file path is empty"))
            }
            AuthDto::KeyFile { path } => AuthMethod::KeyFile {
                path: path.trim().into(),
            },
            AuthDto::Password => AuthMethod::Password,
            AuthDto::Interactive => AuthMethod::Interactive,
        })
    }
}

impl From<&Credential> for CredentialDto {
    fn from(c: &Credential) -> Self {
        Self {
            name: c.name.clone(),
            user: c.user.clone(),
            auth: (&c.auth).into(),
        }
    }
}

impl From<&Profile> for ProfileDto {
    fn from(p: &Profile) -> Self {
        Self {
            name: p.name.clone(),
            protocol: p.protocol.into(),
            host: p.host.clone(),
            port: p.port,
            user: p.user.clone(),
            auth: (&p.auth).into(),
            remote_dir: p.remote_dir.clone(),
            group: p.group.clone(),
            credential: p.credential.clone(),
        }
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl ProfileDto {
    fn into_profile(self) -> ServiceResult<Profile> {
        let protocol = self
            .protocol
            .to_config()
            .ok_or_else(|| ServiceError::invalid("a profile cannot use the local protocol"))?;
        let credential = non_empty(self.credential);
        // A profile using a shared credential ignores its own auth.
        let auth = match credential {
            Some(_) => AuthMethod::Agent,
            None => self.auth.into_method()?,
        };
        let mut p = Profile::new(self.name.trim(), protocol, self.host.trim(), auth);
        p.port = self.port;
        p.user = non_empty(self.user);
        p.remote_dir = non_empty(self.remote_dir);
        p.group = non_empty(self.group);
        p.credential = credential;
        p.validate()?;
        Ok(p)
    }
}

/// CRUD on the profile file plus keychain secrets.
pub struct ProfileStore {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    lock: Mutex<()>,
}

impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            path: path.into(),
            secrets,
            lock: Mutex::new(()),
        }
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        &*self.secrets
    }

    fn load(&self) -> ServiceResult<Config> {
        Ok(Config::load_from(&self.path)?)
    }

    /// Write all profiles and credentials to `path` (no passwords).
    pub fn export_to(&self, path: &std::path::Path) -> ServiceResult<usize> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = self.load()?;
        cfg.save_to(path)?;
        Ok(cfg.list().len() + cfg.credentials().len())
    }

    /// Merge an exported file into the saved profiles.
    pub fn import_from(&self, path: &std::path::Path, overwrite: bool) -> ServiceResult<ImportDto> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if !path.is_file() {
            return Err(oxfer_config::Error::NotFound(path.display().to_string()).into());
        }
        let incoming = Config::load_from(path)?;
        let mut cfg = self.load()?;
        let r = cfg.merge(&incoming, overwrite)?;
        cfg.save_to(&self.path)?;
        Ok(ImportDto {
            added: r.added.len(),
            replaced: r.replaced.len(),
            skipped: r.skipped.len(),
        })
    }

    pub fn list(&self) -> ServiceResult<Vec<ProfileDto>> {
        let mut v: Vec<_> = self.load()?.list().iter().map(ProfileDto::from).collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        Ok(v)
    }

    /// The profile `name` with its shared credential (if any) resolved.
    pub fn get(&self, name: &str) -> ServiceResult<Profile> {
        let cfg = self.load()?;
        let p = cfg
            .get(name)
            .ok_or_else(|| oxfer_config::Error::NotFound(name.into()))?;
        Ok(cfg.resolve(p)?)
    }

    pub fn credential(&self, name: &str) -> ServiceResult<Credential> {
        self.load()?
            .credential(name)
            .cloned()
            .ok_or_else(|| oxfer_config::Error::UnknownCredential(name.into()).into())
    }

    pub fn list_credentials(&self) -> ServiceResult<Vec<CredentialDto>> {
        let mut v: Vec<_> = self
            .load()?
            .credentials()
            .iter()
            .map(CredentialDto::from)
            .collect();
        v.sort_by_key(|c| c.name.to_lowercase());
        Ok(v)
    }

    /// Create (`original` = `None`) or replace the credential `original`
    /// (possibly renaming it; profiles follow). A non-empty `password` is
    /// stored; switching away from password auth deletes the stored one.
    pub fn save_credential(
        &self,
        original: Option<&str>,
        credential: CredentialDto,
        password: Option<SecretString>,
    ) -> ServiceResult<CredentialDto> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let new = Credential::new(
            credential.name.trim(),
            non_empty(credential.user),
            credential.auth.into_method()?,
        );
        let mut cfg = self.load()?;
        let old = match original {
            Some(o) => {
                cfg.replace_credential(o, new.clone())?;
                Some(format!("{}{o}", oxfer_config::CREDENTIAL_SECRET_PREFIX))
            }
            None => {
                cfg.add_credential(new.clone())?;
                None
            }
        };
        let key = new.secret_key();
        let password = password.filter(|p| !p.0.is_empty());
        if new.auth == AuthMethod::Password {
            if let Some(pw) = &password {
                self.secrets.set(&key, &Secret::new(pw.0.clone()))?;
            } else if let Some(old) = old.as_ref().filter(|o| **o != key) {
                if let Some(s) = self.secrets.get(old)? {
                    self.secrets.set(&key, &s)?;
                }
            }
        } else {
            self.secrets.delete(&key)?;
        }
        if let Some(old) = old.filter(|o| *o != key) {
            self.secrets.delete(&old)?;
        }
        cfg.save_to(&self.path)?;
        Ok(CredentialDto::from(&new))
    }

    /// Delete a credential no profile uses, with its stored password.
    pub fn delete_credential(&self, name: &str) -> ServiceResult<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = self.load()?;
        let removed = cfg.remove_credential(name)?;
        cfg.save_to(&self.path)?;
        self.secrets.delete(&removed.secret_key())?;
        Ok(())
    }

    /// Create (`original` = `None`) or replace the profile `original`
    /// (possibly renaming it). A non-empty `password` is stored in the secret
    /// store; switching away from password auth deletes the stored one.
    /// With `copy_from` (a new profile cloned from an existing one) and no new
    /// password, the stored password of `copy_from` is copied.
    pub fn save(
        &self,
        original: Option<&str>,
        profile: ProfileDto,
        password: Option<SecretString>,
        copy_from: Option<&str>,
    ) -> ServiceResult<ProfileDto> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let new = profile.into_profile()?;
        let mut cfg = self.load()?;
        if let Some(c) = &new.credential {
            if cfg.credential(c).is_none() {
                return Err(oxfer_config::Error::UnknownCredential(c.clone()).into());
            }
        }
        if let Some(old) = original {
            cfg.remove(old)?;
        }
        cfg.add(new.clone())?;

        let renamed_from = original.filter(|o| *o != new.name);
        let password = password.filter(|p| !p.0.is_empty());
        if new.credential.is_none() && new.auth == AuthMethod::Password {
            if let Some(pw) = &password {
                self.secrets.set(&new.name, &Secret::new(pw.0.clone()))?;
            } else if let Some(old) = renamed_from.or(copy_from) {
                if let Some(s) = self.secrets.get(old)? {
                    self.secrets.set(&new.name, &s)?;
                }
            }
        } else {
            self.secrets.delete(&new.name)?;
        }
        if let Some(old) = renamed_from {
            self.secrets.delete(old)?;
        }
        cfg.save_to(&self.path)?;
        Ok(ProfileDto::from(&new))
    }

    pub fn delete(&self, name: &str) -> ServiceResult<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = self.load()?;
        cfg.remove(name)?;
        cfg.save_to(&self.path)?;
        self.secrets.delete(name)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxfer_config::MemoryStore;

    fn dto(name: &str, auth: AuthDto) -> ProfileDto {
        ProfileDto {
            name: name.into(),
            protocol: ProtocolDto::Sftp,
            host: "example.com".into(),
            port: Some(2222),
            user: Some("bob".into()),
            auth,
            remote_dir: Some(" ".into()),
            group: Some(" Work ".into()),
            credential: None,
        }
    }

    #[test]
    fn crud_with_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemoryStore::default());
        let store = ProfileStore::new(dir.path().join("config.toml"), secrets.clone());
        assert!(store.list().unwrap().is_empty());

        let saved = store
            .save(
                None,
                dto("web", AuthDto::Password),
                Some(SecretString("pw".into())),
                None,
            )
            .unwrap();
        assert_eq!(saved.remote_dir, None);
        assert_eq!(saved.group.as_deref(), Some("Work"));
        assert_eq!(secrets.get("web").unwrap().unwrap().expose(), "pw");
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("pw\""), "password leaked into file: {text}");

        // duplicate
        let e = store
            .save(None, dto("web", AuthDto::Agent), None, None)
            .unwrap_err();
        assert_eq!(e.code, "alreadyExists");

        // duplicate: copy_from carries the password over, original stays intact
        store
            .save(None, dto("web-copy", AuthDto::Password), None, Some("web"))
            .unwrap();
        assert_eq!(secrets.get("web-copy").unwrap().unwrap().expose(), "pw");
        assert_eq!(secrets.get("web").unwrap().unwrap().expose(), "pw");
        store.delete("web-copy").unwrap();
        assert!(secrets.get("web-copy").unwrap().is_none());

        // export carries no password; import merges and skips existing names
        let file = dir.path().join("export.toml");
        assert_eq!(store.export_to(&file).unwrap(), 1);
        assert!(!std::fs::read_to_string(&file).unwrap().contains("pw"));
        let r = store.import_from(&file, false).unwrap();
        assert_eq!((r.added, r.replaced, r.skipped), (0, 0, 1));
        assert_eq!(
            store
                .import_from(&dir.path().join("nope.toml"), false)
                .unwrap_err()
                .code,
            "notFound"
        );

        // rename keeps the password
        store
            .save(Some("web"), dto("web2", AuthDto::Password), None, None)
            .unwrap();
        assert!(secrets.get("web").unwrap().is_none());
        assert_eq!(secrets.get("web2").unwrap().unwrap().expose(), "pw");
        assert_eq!(store.list().unwrap()[0].name, "web2");

        // switching to agent removes the password
        store
            .save(Some("web2"), dto("web2", AuthDto::Agent), None, None)
            .unwrap();
        assert!(secrets.get("web2").unwrap().is_none());

        // invalid input
        let mut bad = dto("x", AuthDto::Agent);
        bad.host = "".into();
        assert_eq!(
            store.save(None, bad, None, None).unwrap_err().code,
            "invalid"
        );
        let mut local = dto("x", AuthDto::Agent);
        local.protocol = ProtocolDto::Local;
        assert_eq!(
            store.save(None, local, None, None).unwrap_err().code,
            "invalid"
        );

        store.delete("web2").unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.delete("web2").unwrap_err().code, "notFound");
    }

    #[test]
    fn shared_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemoryStore::default());
        let store = ProfileStore::new(dir.path().join("config.toml"), secrets.clone());
        let cred = CredentialDto {
            name: "deploy".into(),
            user: Some("deployer".into()),
            auth: AuthDto::Password,
        };
        store
            .save_credential(None, cred.clone(), Some(SecretString("s3cret".into())))
            .unwrap();
        assert_eq!(
            secrets.get("credential:deploy").unwrap().unwrap().expose(),
            "s3cret"
        );

        // a profile with an unknown credential is refused
        let mut p = dto("web", AuthDto::Password);
        p.credential = Some("nope".into());
        assert_eq!(
            store.save(None, p.clone(), None, None).unwrap_err().code,
            "notFound"
        );

        // using the credential: resolved on get, own password not stored
        p.credential = Some("deploy".into());
        store
            .save(None, p, Some(SecretString("own".into())), None)
            .unwrap();
        assert!(secrets.get("web").unwrap().is_none());
        let resolved = store.get("web").unwrap();
        assert_eq!(resolved.user.as_deref(), Some("deployer"));
        assert_eq!(resolved.auth, AuthMethod::Password);
        assert_eq!(resolved.secret_key(), "credential:deploy");

        // rename moves the password and updates the profile
        let mut renamed = cred.clone();
        renamed.name = "ci".into();
        store
            .save_credential(Some("deploy"), renamed, None)
            .unwrap();
        assert!(secrets.get("credential:deploy").unwrap().is_none());
        assert_eq!(
            secrets.get("credential:ci").unwrap().unwrap().expose(),
            "s3cret"
        );
        assert_eq!(store.list().unwrap()[0].credential.as_deref(), Some("ci"));

        // in use: refused; afterwards deleted with its password
        assert_eq!(store.delete_credential("ci").unwrap_err().code, "inUse");
        store.delete("web").unwrap();
        store.delete_credential("ci").unwrap();
        assert!(secrets.get("credential:ci").unwrap().is_none());
        assert!(store.list_credentials().unwrap().is_empty());
    }

    #[test]
    fn json_shape() {
        let v = serde_json::to_value(dto(
            "a",
            AuthDto::KeyFile {
                path: "~/.ssh/id".into(),
            },
        ))
        .unwrap();
        assert_eq!(v["auth"]["method"], "keyFile");
        assert_eq!(v["auth"]["path"], "~/.ssh/id");
        assert!(v.get("remoteDir").is_some());
    }
}
