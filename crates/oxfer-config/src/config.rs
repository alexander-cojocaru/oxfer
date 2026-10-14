use crate::credential::Credential;
use crate::profile::Profile;
use crate::{Error, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What [`Config::merge`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Names of profiles and credentials that were new.
    pub added: Vec<String>,
    /// Names that already existed and were replaced (`overwrite`).
    pub replaced: Vec<String>,
    /// Names that already existed and were left alone.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credentials: Vec<Credential>,
}

impl Config {
    /// `<config_dir>/oxfer/config.toml`.
    pub fn default_path() -> Result<PathBuf> {
        ProjectDirs::from("", "", "oxfer")
            .map(|d| d.config_dir().join("config.toml"))
            .ok_or(Error::NoConfigDir)
    }

    pub fn load() -> Result<Self> {
        Self::load_from(Self::default_path()?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(Self::default_path()?)
    }

    /// Load a config; a missing file yields an empty config.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(Error::Io {
                    path: path.into(),
                    source,
                })
            }
        };
        let parse_err = |message: String| Error::Parse {
            path: path.into(),
            message,
        };
        let cfg: Config = toml::from_str(&text).map_err(|e| parse_err(e.to_string()))?;
        let mut checked = Config::default();
        for c in &cfg.credentials {
            checked
                .add_credential(c.clone())
                .map_err(|e| parse_err(e.to_string()))?;
        }
        for p in &cfg.profiles {
            checked
                .add(p.clone())
                .map_err(|e| parse_err(e.to_string()))?;
        }
        Ok(cfg)
    }

    /// Atomically write (temp file + rename); mode 0600 on unix.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let io = |source| Error::Io {
            path: path.into(),
            source,
        };
        let text = toml::to_string_pretty(self).map_err(|e| Error::Serialize(e.to_string()))?;
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".tmp{}", std::process::id()));
        let tmp = PathBuf::from(tmp);
        let write = || -> std::io::Result<()> {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })
    }

    /// Merge the credentials and profiles of `other` (e.g. an exported file)
    /// into `self`. Existing names are kept unless `overwrite`. Secrets are
    /// never part of a config, so imported password entries have none yet.
    pub fn merge(&mut self, other: &Config, overwrite: bool) -> Result<MergeReport> {
        let mut report = MergeReport::default();
        for c in &other.credentials {
            match self.credentials.iter().position(|x| x.name == c.name) {
                None => {
                    self.add_credential(c.clone())?;
                    report.added.push(c.name.clone());
                }
                Some(i) if overwrite => {
                    c.validate()?;
                    self.credentials[i] = c.clone();
                    report.replaced.push(c.name.clone());
                }
                Some(_) => report.skipped.push(c.name.clone()),
            }
        }
        for p in &other.profiles {
            if let Some(name) = &p.credential {
                if self.credential(name).is_none() {
                    return Err(Error::UnknownCredential(name.clone()));
                }
            }
            match self.profiles.iter().position(|x| x.name == p.name) {
                None => {
                    self.add(p.clone())?;
                    report.added.push(p.name.clone());
                }
                Some(i) if overwrite => {
                    p.validate()?;
                    self.profiles[i] = p.clone();
                    report.replaced.push(p.name.clone());
                }
                Some(_) => report.skipped.push(p.name.clone()),
            }
        }
        Ok(report)
    }

    pub fn list(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn add(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        if self.get(&profile.name).is_some() {
            return Err(Error::Duplicate(profile.name));
        }
        self.profiles.push(profile);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<Profile> {
        let i = self
            .profiles
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| Error::NotFound(name.into()))?;
        Ok(self.profiles.remove(i))
    }

    /// `profile` with the user and authentication of its shared credential
    /// filled in (the credential's user wins; the profile's is the fallback).
    /// Profiles without a credential are returned unchanged.
    pub fn resolve(&self, profile: &Profile) -> Result<Profile> {
        let mut p = profile.clone();
        if let Some(name) = &profile.credential {
            let c = self
                .credential(name)
                .ok_or_else(|| Error::UnknownCredential(name.clone()))?;
            p.auth = c.auth.clone();
            p.user = c.user.clone().or(p.user);
        }
        Ok(p)
    }

    pub fn credentials(&self) -> &[Credential] {
        &self.credentials
    }

    pub fn credential(&self, name: &str) -> Option<&Credential> {
        self.credentials.iter().find(|c| c.name == name)
    }

    pub fn add_credential(&mut self, credential: Credential) -> Result<()> {
        credential.validate()?;
        if self.credential(&credential.name).is_some() {
            return Err(Error::DuplicateCredential(credential.name));
        }
        self.credentials.push(credential);
        Ok(())
    }

    /// Replace the credential `original` (possibly renaming it; profiles
    /// that use it follow the new name).
    pub fn replace_credential(&mut self, original: &str, credential: Credential) -> Result<()> {
        credential.validate()?;
        let i = self
            .credentials
            .iter()
            .position(|c| c.name == original)
            .ok_or_else(|| Error::UnknownCredential(original.into()))?;
        if credential.name != original && self.credential(&credential.name).is_some() {
            return Err(Error::DuplicateCredential(credential.name));
        }
        for p in &mut self.profiles {
            if p.credential.as_deref() == Some(original) {
                p.credential = Some(credential.name.clone());
            }
        }
        self.credentials[i] = credential;
        Ok(())
    }

    /// Remove a credential no profile uses.
    pub fn remove_credential(&mut self, name: &str) -> Result<Credential> {
        let users: Vec<&str> = self
            .profiles
            .iter()
            .filter(|p| p.credential.as_deref() == Some(name))
            .map(|p| p.name.as_str())
            .collect();
        if !users.is_empty() {
            return Err(Error::CredentialInUse {
                name: name.into(),
                profiles: users.join(", "),
            });
        }
        let i = self
            .credentials
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| Error::UnknownCredential(name.into()))?;
        Ok(self.credentials.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{AuthMethod, Protocol};

    #[test]
    fn merge_adds_skips_and_overwrites() {
        let mut a = Config::default();
        a.add(p("a")).unwrap();
        let mut b = Config::default();
        let mut a2 = p("a");
        a2.host = "other".into();
        b.add(a2).unwrap();
        b.add(p("b")).unwrap();

        let r = a.merge(&b, false).unwrap();
        assert_eq!((r.added, r.skipped), (vec!["b".into()], vec!["a".into()]));
        assert_ne!(a.get("a").unwrap().host, "other");

        let r = a.merge(&b, true).unwrap();
        assert_eq!(r.replaced.len(), 2);
        assert_eq!(a.get("a").unwrap().host, "other");
        assert_eq!(a.list().len(), 2);
    }

    fn p(name: &str) -> Profile {
        Profile::new(name, Protocol::Sftp, "example.com", AuthMethod::Agent)
    }

    #[test]
    fn crud_and_uniqueness() {
        let mut c = Config::default();
        c.add(p("a")).unwrap();
        c.add(p("b")).unwrap();
        assert!(matches!(c.add(p("a")), Err(Error::Duplicate(_))));
        assert!(matches!(c.add(p("")), Err(Error::Invalid(_))));
        let mut bad = p("c");
        bad.host = " ".into();
        assert!(c.add(bad).is_err());
        assert_eq!(c.list().len(), 2);
        assert!(c.get("b").is_some());
        c.remove("a").unwrap();
        assert!(matches!(c.remove("a"), Err(Error::NotFound(_))));
        assert_eq!(c.list().len(), 1);
    }

    #[test]
    fn roundtrip_all_variants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut c = Config::default();
        let mut a = p("a");
        a.port = Some(2200);
        a.user = Some("u".into());
        a.remote_dir = Some("/srv".into());
        a.proxy_jump = Some("bastion".into());
        a.group = Some("Work".into());
        a.auth = AuthMethod::KeyFile {
            path: "~/.ssh/id".into(),
        };
        c.add(a).unwrap();
        for (n, proto, auth) in [
            ("b", Protocol::Ftp, AuthMethod::Password),
            ("c", Protocol::FtpsExplicit, AuthMethod::Interactive),
            ("d", Protocol::FtpsImplicit, AuthMethod::Agent),
        ] {
            c.add(Profile::new(n, proto, "h", auth)).unwrap();
        }
        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), c);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("method = \"key-file\""), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // overwrite works and leaves no temp files
        c.save_to(&path).unwrap();
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn credentials_crud_resolve_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.add_credential(Credential::new(
            "deploy",
            Some("deployer".into()),
            AuthMethod::Password,
        ))
        .unwrap();
        assert!(matches!(
            c.add_credential(Credential::new("deploy", None, AuthMethod::Agent)),
            Err(Error::DuplicateCredential(_))
        ));
        let mut web = p("web");
        web.user = Some("ignored".into());
        web.credential = Some("deploy".into());
        c.add(web.clone()).unwrap();
        c.add(p("plain")).unwrap();

        let r = c.resolve(&web).unwrap();
        assert_eq!(r.user.as_deref(), Some("deployer"));
        assert_eq!(r.auth, AuthMethod::Password);
        assert_eq!(r.secret_key(), "credential:deploy");
        assert_eq!(
            c.resolve(c.get("plain").unwrap()).unwrap().secret_key(),
            "plain"
        );

        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), c);

        // in use: refused; rename: references follow
        assert!(matches!(
            c.remove_credential("deploy"),
            Err(Error::CredentialInUse { ref profiles, .. }) if profiles == "web"
        ));
        c.replace_credential("deploy", Credential::new("ci", None, AuthMethod::Agent))
            .unwrap();
        assert_eq!(c.get("web").unwrap().credential.as_deref(), Some("ci"));
        // no user on the credential: the profile's is used
        assert_eq!(
            c.resolve(c.get("web").unwrap()).unwrap().user.as_deref(),
            Some("ignored")
        );
        c.remove("web").unwrap();
        c.remove_credential("ci").unwrap();
        assert!(c.credentials().is_empty());

        let mut dangling = p("x");
        dangling.credential = Some("nope".into());
        assert!(matches!(
            c.resolve(&dangling),
            Err(Error::UnknownCredential(_))
        ));
        assert!(c.add(p("credential:x")).is_err());
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load_from(dir.path().join("nope.toml")).unwrap();
        assert!(c.list().is_empty());
    }

    #[test]
    fn malformed_files_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        let ok =
            "[[profiles]]\nname=\"x\"\nprotocol=\"sftp\"\nhost=\"h\"\nauth={method=\"agent\"}\n";
        let cases = [
            "this is not toml [".to_string(),
            "[[profiles]]\nname = \"x\"\n".to_string(),
            ok.replace("sftp", "gopher"),
            format!("{ok}port=70000\n"),
            format!("{ok}{ok}"),
        ];
        for text in cases {
            std::fs::write(&path, &text).unwrap();
            let e = Config::load_from(&path).unwrap_err();
            assert!(matches!(e, Error::Parse { .. }), "{text}: {e}");
            assert!(e.to_string().contains("c.toml"));
        }
    }
}
