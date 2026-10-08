use anyhow::{bail, Result};

/// A `[user@]host[:path]` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSpec {
    pub user: Option<String>,
    pub host: String,
    pub path: String,
}

impl std::str::FromStr for RemoteSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (user, rest) = match s.split_once('@') {
            Some(("", _)) => bail!("empty user in {s:?}"),
            Some((u, r)) => (Some(u.to_string()), r),
            None => (None, s),
        };
        let (host, path) = if let Some(stripped) = rest.strip_prefix('[') {
            // [ipv6]:path
            let Some((h, after)) = stripped.split_once(']') else {
                bail!("unterminated '[' in {s:?}");
            };
            (h, after.strip_prefix(':').unwrap_or(""))
        } else {
            rest.split_once(':').unwrap_or((rest, ""))
        };
        if host.is_empty() {
            bail!("missing host in {s:?}");
        }
        Ok(Self {
            user,
            host: host.to_string(),
            path: if path.is_empty() {
                ".".into()
            } else {
                path.into()
            },
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
    fn parses() {
        assert_eq!(
            p("bob@host:/tmp/a"),
            RemoteSpec {
                user: Some("bob".into()),
                host: "host".into(),
                path: "/tmp/a".into()
            }
        );
        assert_eq!(p("host").path, ".");
        assert_eq!(p("host:").path, ".");
        assert_eq!(p("host:rel/x").user, None);
        assert_eq!(p("u@[::1]:/x").host, "::1");
    }

    #[test]
    fn rejects_empty_host() {
        assert!("@:/x".parse::<RemoteSpec>().is_err());
        assert!(":/x".parse::<RemoteSpec>().is_err());
    }
}
