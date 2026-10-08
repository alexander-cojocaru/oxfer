use crate::profile::Profile;
use crate::{Error, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    profiles: Vec<Profile>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{AuthMethod, Protocol};

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
