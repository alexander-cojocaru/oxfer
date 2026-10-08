//! `ProxyJump` chains: `[user@]host[:port][,[user@]host[:port]...]`.

use oxfer_sftp::{Auth, ConnectOptions};

use crate::ssh_config::SshConfig;
use crate::{Error, Result};

/// Build the connect options of the jump chain for `proxy`, innermost
/// (first) hop first, linked through [`ConnectOptions::jump`]. The returned
/// options are the *last* hop; assign them to the target's `jump` field.
///
/// Each hop is resolved through `ssh` (so aliases work) and authenticates with
/// the ssh-agent, or the first existing `IdentityFile` when `ssh` names one.
pub fn jump_chain(proxy: &str, ssh: Option<&SshConfig>) -> Result<Option<Box<ConnectOptions>>> {
    let mut previous: Option<Box<ConnectOptions>> = None;
    for hop in proxy.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let (user, hostport) = match hop.split_once('@') {
            Some((u, r)) if !u.is_empty() => (Some(u.to_string()), r),
            Some(_) => return Err(Error::InvalidJump(hop.into())),
            None => (None, hop),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (
                h,
                Some(
                    p.parse::<u16>()
                        .map_err(|_| Error::InvalidJump(hop.into()))?,
                ),
            ),
            None => (hostport, None),
        };
        if host.is_empty() {
            return Err(Error::InvalidJump(hop.into()));
        }
        let resolved = ssh.map(|s| s.resolve(host));
        let user = user
            .or_else(|| resolved.as_ref().and_then(|r| r.user.clone()))
            .or_else(|| std::env::var("USER").ok())
            .or_else(|| std::env::var("USERNAME").ok())
            .ok_or_else(|| Error::InvalidJump(format!("{hop}: no user")))?;
        let auth = resolved
            .as_ref()
            .and_then(|r| r.identity_files.iter().find(|p| p.exists()))
            .map_or(Auth::Agent, |path| Auth::Key {
                path: path.clone(),
                passphrase: None,
            });
        let hostname = resolved
            .as_ref()
            .map_or(host.to_string(), |r| r.hostname.clone());
        let mut opts = ConnectOptions::new(hostname, user, auth);
        opts.port = port.or(resolved.and_then(|r| r.port)).unwrap_or(22);
        opts.jump = previous.take();
        previous = Some(Box::new(opts));
    }
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_and_chained_hops() {
        let one = jump_chain("bob@bastion:2222", None).unwrap().unwrap();
        assert_eq!(
            (one.host.as_str(), one.port, one.user.as_str()),
            ("bastion", 2222, "bob")
        );
        assert!(one.jump.is_none());

        let chain = jump_chain("a@h1,b@h2:23", None).unwrap().unwrap();
        // the last hop is returned; its own jump is the first hop
        assert_eq!((chain.host.as_str(), chain.port), ("h2", 23));
        assert_eq!(chain.jump.as_ref().unwrap().host, "h1");
        assert!(jump_chain("", None).unwrap().is_none());
    }

    #[test]
    fn resolves_aliases() {
        let cfg = SshConfig::parse("Host gw\n HostName 10.1.1.1\n User ops\n Port 2200\n");
        let o = jump_chain("gw", Some(&cfg)).unwrap().unwrap();
        assert_eq!(
            (o.host.as_str(), o.port, o.user.as_str()),
            ("10.1.1.1", 2200, "ops")
        );
    }

    #[test]
    fn rejects_bad_hops() {
        assert!(jump_chain("@h", None).is_err());
        assert!(jump_chain("u@h:xx", None).is_err());
        assert!(jump_chain("u@:22", None).is_err());
    }
}
