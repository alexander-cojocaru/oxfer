use anyhow::{bail, Result};
use oxfer_config::Protocol;

/// A remote location: `[user@]host[:path]`, a URL (`sftp://`, `ftp://`,
/// `ftps://` = implicit TLS, `ftpes://` = explicit TLS) or `@profile[:path]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSpec {
    pub protocol: Protocol,
    pub profile: Option<String>,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
}

impl RemoteSpec {
    /// Whether the user gave an explicit path (as opposed to the default).
    pub fn has_path(&self) -> bool {
        self.path != "."
    }
}

fn split_user(s: &str, whole: &str) -> Result<(Option<String>, String)> {
    match s.rsplit_once('@') {
        Some(("", _)) => bail!("empty user in {whole:?}"),
        Some((u, r)) => Ok((Some(u.to_string()), r.to_string())),
        None => Ok((None, s.to_string())),
    }
}

impl std::str::FromStr for RemoteSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if let Some(rest) = s.strip_prefix('@') {
            let (name, path) = rest.split_once(':').unwrap_or((rest, ""));
            if name.is_empty() {
                bail!("empty profile name in {s:?}");
            }
            return Ok(Self {
                protocol: Protocol::Sftp, // replaced by the profile's own protocol
                profile: Some(name.into()),
                user: None,
                host: String::new(),
                port: None,
                path: if path.is_empty() {
                    ".".into()
                } else {
                    path.into()
                },
            });
        }

        if let Some((scheme, rest)) = s.split_once("://") {
            let protocol = match scheme {
                "sftp" | "ssh" => Protocol::Sftp,
                "ftp" => Protocol::Ftp,
                "ftps" => Protocol::FtpsImplicit,
                "ftpes" => Protocol::FtpsExplicit,
                other => bail!("unknown scheme {other:?} in {s:?}"),
            };
            let (authority, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            let (user, hostport) = split_user(authority, s)?;
            let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
                let Some((h, after)) = v6.split_once(']') else {
                    bail!("unterminated '[' in {s:?}");
                };
                (h.to_string(), after.strip_prefix(':'))
            } else {
                match hostport.rsplit_once(':') {
                    Some((h, p)) => (h.to_string(), Some(p)),
                    None => (hostport.clone(), None),
                }
            };
            let port = port
                .filter(|p| !p.is_empty())
                .map(|p| p.parse::<u16>())
                .transpose()
                .map_err(|_| anyhow::anyhow!("invalid port in {s:?}"))?;
            if host.is_empty() {
                bail!("missing host in {s:?}");
            }
            let default_path = if protocol == Protocol::Sftp { "." } else { "/" };
            return Ok(Self {
                protocol,
                profile: None,
                user,
                host,
                port,
                path: if path.is_empty() {
                    default_path.into()
                } else {
                    path.into()
                },
            });
        }

        let (user, rest) = split_user(s, s)?;
        let (host, path) = if let Some(stripped) = rest.strip_prefix('[') {
            let Some((h, after)) = stripped.split_once(']') else {
                bail!("unterminated '[' in {s:?}");
            };
            (
                h.to_string(),
                after.strip_prefix(':').unwrap_or("").to_string(),
            )
        } else {
            match rest.split_once(':') {
                Some((h, p)) => (h.to_string(), p.to_string()),
                None => (rest.clone(), String::new()),
            }
        };
        if host.is_empty() {
            bail!("missing host in {s:?}");
        }
        Ok(Self {
            protocol: Protocol::Sftp,
            profile: None,
            user,
            host,
            port: None,
            path: if path.is_empty() { ".".into() } else { path },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> RemoteSpec {
        s.parse().unwrap()
    }

    #[test]
    fn parses_scp_style() {
        let r = p("bob@host:/tmp/a");
        assert_eq!(
            (r.user.as_deref(), r.host.as_str(), r.path.as_str()),
            (Some("bob"), "host", "/tmp/a")
        );
        assert_eq!(r.protocol, Protocol::Sftp);
        assert_eq!(p("host").path, ".");
        assert_eq!(p("host:").path, ".");
        assert_eq!(p("host:rel/x").user, None);
        assert_eq!(p("u@[::1]:/x").host, "::1");
    }

    #[test]
    fn parses_urls() {
        let r = p("ftp://anon@example.com:2121/pub/x");
        assert_eq!(r.protocol, Protocol::Ftp);
        assert_eq!(
            (r.user.as_deref(), r.host.as_str(), r.port, r.path.as_str()),
            (Some("anon"), "example.com", Some(2121), "/pub/x")
        );
        assert_eq!(p("ftp://example.com").path, "/");
        assert_eq!(p("ftps://example.com").protocol, Protocol::FtpsImplicit);
        assert_eq!(p("ftpes://example.com/a").protocol, Protocol::FtpsExplicit);
        let s = p("sftp://u@h:2222");
        assert_eq!((s.port, s.path.as_str()), (Some(2222), "."));
        assert_eq!(p("sftp://[::1]:22/x").host, "::1");
    }

    #[test]
    fn parses_profiles() {
        let r = p("@prod:/srv");
        assert_eq!(
            (r.profile.as_deref(), r.path.as_str()),
            (Some("prod"), "/srv")
        );
        assert_eq!(p("@prod").path, ".");
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "@:/x",
            ":/x",
            "@",
            "gopher://h",
            "ftp://",
            "sftp://h:notaport",
            "sftp://@h",
        ] {
            assert!(bad.parse::<RemoteSpec>().is_err(), "{bad}");
        }
    }
}
