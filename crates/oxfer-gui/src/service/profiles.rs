//! Saved connection profiles (wrapping `oxfer_config`). Passwords go to the
//! injected [`SecretStore`] (the OS keychain in the app), never to the file.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use oxfer_config::{AuthMethod, Config, Profile, Secret, SecretStore};
use serde::{Deserialize, Serialize};

use super::dto::ProtocolDto;
use super::error::{ServiceError, ServiceResult};
use super::prompt::SecretString;

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
}

impl From<&Profile> for ProfileDto {
    fn from(p: &Profile) -> Self {
        Self {
            name: p.name.clone(),
            protocol: p.protocol.into(),
            host: p.host.clone(),
            port: p.port,
            user: p.user.clone(),
            auth: match &p.auth {
                AuthMethod::Agent => AuthDto::Agent,
                AuthMethod::KeyFile { path } => AuthDto::KeyFile {
                    path: path.to_string_lossy().into_owned(),
                },
                AuthMethod::Password => AuthDto::Password,
                AuthMethod::Interactive => AuthDto::Interactive,
            },
            remote_dir: p.remote_dir.clone(),
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
        let auth = match self.auth {
            AuthDto::Agent => AuthMethod::Agent,
            AuthDto::KeyFile { path } if path.trim().is_empty() => {
                return Err(ServiceError::invalid("key file path is empty"))
            }
            AuthDto::KeyFile { path } => AuthMethod::KeyFile { path: path.into() },
            AuthDto::Password => AuthMethod::Password,
            AuthDto::Interactive => AuthMethod::Interactive,
        };
        let mut p = Profile::new(self.name.trim(), protocol, self.host.trim(), auth);
        p.port = self.port;
        p.user = non_empty(self.user);
        p.remote_dir = non_empty(self.remote_dir);
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

    pub fn list(&self) -> ServiceResult<Vec<ProfileDto>> {
        let mut v: Vec<_> = self.load()?.list().iter().map(ProfileDto::from).collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        Ok(v)
    }

    pub fn get(&self, name: &str) -> ServiceResult<Profile> {
        self.load()?
            .get(name)
            .cloned()
            .ok_or_else(|| oxfer_config::Error::NotFound(name.into()).into())
    }

    /// Create (`original` = `None`) or replace the profile `original`
    /// (possibly renaming it). A non-empty `password` is stored in the secret
    /// store; switching away from password auth deletes the stored one.
    pub fn save(
        &self,
        original: Option<&str>,
        profile: ProfileDto,
        password: Option<SecretString>,
    ) -> ServiceResult<ProfileDto> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let new = profile.into_profile()?;
        let mut cfg = self.load()?;
        if let Some(old) = original {
            cfg.remove(old)?;
        }
        cfg.add(new.clone())?;

        let renamed_from = original.filter(|o| *o != new.name);
        let password = password.filter(|p| !p.0.is_empty());
        if new.auth == AuthMethod::Password {
            if let Some(pw) = &password {
                self.secrets.set(&new.name, &Secret::new(pw.0.clone()))?;
            } else if let Some(old) = renamed_from {
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
            )
            .unwrap();
        assert_eq!(saved.remote_dir, None);
        assert_eq!(secrets.get("web").unwrap().unwrap().expose(), "pw");
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("pw\""), "password leaked into file: {text}");

        // duplicate
        let e = store
            .save(None, dto("web", AuthDto::Agent), None)
            .unwrap_err();
        assert_eq!(e.code, "alreadyExists");

        // rename keeps the password
        store
            .save(Some("web"), dto("web2", AuthDto::Password), None)
            .unwrap();
        assert!(secrets.get("web").unwrap().is_none());
        assert_eq!(secrets.get("web2").unwrap().unwrap().expose(), "pw");
        assert_eq!(store.list().unwrap()[0].name, "web2");

        // switching to agent removes the password
        store
            .save(Some("web2"), dto("web2", AuthDto::Agent), None)
            .unwrap();
        assert!(secrets.get("web2").unwrap().is_none());

        // invalid input
        let mut bad = dto("x", AuthDto::Agent);
        bad.host = "".into();
        assert_eq!(store.save(None, bad, None).unwrap_err().code, "invalid");
        let mut local = dto("x", AuthDto::Agent);
        local.protocol = ProtocolDto::Local;
        assert_eq!(store.save(None, local, None).unwrap_err().code, "invalid");

        store.delete("web2").unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.delete("web2").unwrap_err().code, "notFound");
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
