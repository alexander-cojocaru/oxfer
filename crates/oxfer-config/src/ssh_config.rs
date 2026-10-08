//! Minimal OpenSSH client config reader (`~/.ssh/config`).
//!
//! Supports `Host` patterns (`*`, `?`, `!` negation), first-match-wins per
//! keyword, `HostName` (with `%h`), `User`, `Port`, `IdentityFile`
//! (accumulating), `ProxyJump`, `IdentitiesOnly` and `Include` with globbing.
//! `Match` blocks are skipped and unknown keywords ignored.

use directories::BaseDirs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MAX_INCLUDE_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    /// `HostName`, or the alias itself.
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<PathBuf>,
    pub proxy_jump: Option<String>,
    pub identities_only: bool,
}

#[derive(Debug, Clone)]
struct Directive {
    /// `None` for an (unsupported) `Match` block, which never applies.
    patterns: Option<Rc<Vec<String>>>,
    key: String,
    value: String,
}

#[derive(Debug, Clone, Default)]
pub struct SshConfig {
    directives: Vec<Directive>,
    home: Option<PathBuf>,
}

impl SshConfig {
    /// Default location: `~/.ssh/config`.
    pub fn default_path() -> Option<PathBuf> {
        BaseDirs::new().map(|d| d.home_dir().join(".ssh").join("config"))
    }

    /// Load and parse a config file. A missing file yields an empty config.
    /// Relative `Include`s resolve against `~/.ssh`, like OpenSSH.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut cfg = Self::empty();
        match std::fs::read_to_string(path) {
            Ok(text) => cfg.parse_text(&text, 0),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(cfg)
    }

    /// Parse config text (includes are still read from disk).
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::empty();
        cfg.parse_text(text, 0);
        cfg
    }

    fn empty() -> Self {
        Self {
            directives: Vec::new(),
            home: BaseDirs::new().map(|d| d.home_dir().to_path_buf()),
        }
    }

    /// Override the home directory used for `~` expansion and relative includes.
    pub fn set_home(&mut self, home: impl Into<PathBuf>) {
        self.home = Some(home.into());
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    fn parse_text(&mut self, text: &str, depth: usize) {
        let mut ctx = Some(Rc::new(vec!["*".to_string()]));
        self.parse_into(text, depth, &mut ctx);
    }

    fn parse_into(&mut self, text: &str, depth: usize, ctx: &mut Option<Rc<Vec<String>>>) {
        for line in text.lines() {
            let Some((key, args)) = split_line(line) else {
                continue;
            };
            match key.as_str() {
                "host" => *ctx = Some(Rc::new(args)),
                "match" => *ctx = None,
                "include" => {
                    if depth >= MAX_INCLUDE_DEPTH {
                        continue;
                    }
                    let saved = ctx.clone();
                    for arg in &args {
                        for file in self.include_files(arg) {
                            if let Ok(t) = std::fs::read_to_string(&file) {
                                self.parse_into(&t, depth + 1, ctx);
                            }
                        }
                    }
                    *ctx = saved;
                }
                _ => self.directives.push(Directive {
                    patterns: ctx.clone(),
                    key,
                    value: args.first().cloned().unwrap_or_default(),
                }),
            }
        }
    }

    fn include_files(&self, pattern: &str) -> Vec<PathBuf> {
        let mut p = expand_tilde(Path::new(pattern), self.home());
        if p.is_relative() {
            if let Some(h) = &self.home {
                p = h.join(".ssh").join(p);
            }
        }
        glob::glob(&p.to_string_lossy())
            .map(|g| g.flatten().filter(|f| f.is_file()).collect())
            .unwrap_or_default()
    }

    /// Resolve an alias to connection settings (first match wins per keyword;
    /// `IdentityFile` accumulates).
    pub fn resolve(&self, alias: &str) -> ResolvedHost {
        let (mut hostname, mut user, mut port, mut proxy, mut only) =
            (None, None, None, None, None);
        let mut ids = Vec::new();
        for d in &self.directives {
            let Some(pats) = &d.patterns else { continue };
            if !host_matches(pats, alias) {
                continue;
            }
            match d.key.as_str() {
                "hostname" => hostname = hostname.or(Some(d.value.replace("%h", alias))),
                "user" => user = user.or(Some(d.value.clone())),
                "port" => {
                    if port.is_none() {
                        port = d.value.parse::<u16>().ok();
                    }
                }
                "identityfile" if !d.value.eq_ignore_ascii_case("none") => {
                    ids.push(expand_tilde(Path::new(&d.value), self.home()));
                }
                "proxyjump" => proxy = proxy.or(Some(d.value.clone())),
                "identitiesonly" => only = only.or(Some(is_yes(&d.value))),
                _ => {}
            }
        }
        ResolvedHost {
            hostname: hostname.unwrap_or_else(|| alias.to_string()),
            user,
            port,
            identity_files: ids,
            proxy_jump: proxy.filter(|p| !p.eq_ignore_ascii_case("none")),
            identities_only: only.unwrap_or(false),
        }
    }
}

fn is_yes(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "yes" | "true")
}

/// Expand a leading `~` / `~/`.
pub fn expand_tilde(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(h)) => h.join(rest),
        _ => path.to_path_buf(),
    }
}

/// Split a config line into a lowercased keyword and unquoted arguments.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let key = line[..end].to_ascii_lowercase();
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    let (mut args, mut cur, mut quoted, mut has) = (Vec::new(), String::new(), false, false);
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '#' if !quoted => break,
            c if c.is_whitespace() && !quoted => {
                if has {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        args.push(cur);
    }
    Some((key, args))
}

/// A pattern list matches if some positive pattern matches and no negated one does.
fn host_matches(patterns: &[String], host: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        match p.strip_prefix('!') {
            Some(neg) if wildcard(neg, host) => return false,
            Some(_) => {}
            None => hit |= wildcard(p, host),
        }
    }
    hit
}

fn wildcard(pat: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(text: &str) -> SshConfig {
        let mut c = SshConfig::parse(text);
        c.set_home("/home/u");
        c
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*", "anything"));
        assert!(wildcard("*.example.com", "a.example.com"));
        assert!(!wildcard("*.example.com", "example.org"));
        assert!(wildcard("web?", "web1"));
        assert!(!wildcard("web?", "web12"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        assert!(!wildcard("a*b*c", "aXXbYY"));
        assert!(wildcard("", ""));
    }

    #[test]
    fn negation() {
        let pats: Vec<String> = ["*.example.com", "!secret.example.com"]
            .map(String::from)
            .into();
        assert!(host_matches(&pats, "a.example.com"));
        assert!(!host_matches(&pats, "secret.example.com"));
        let only_neg = vec!["!foo".to_string()];
        assert!(!host_matches(&only_neg, "bar"));
    }

    #[test]
    fn basic_resolution() {
        let c = cfg("Host web\n  HostName 10.0.0.5\n  User bob\n  Port 2222\n  IdentityFile ~/.ssh/web_key\n  ProxyJump jump\n  IdentitiesOnly yes\n");
        let r = c.resolve("web");
        assert_eq!(r.hostname, "10.0.0.5");
        assert_eq!(r.user.as_deref(), Some("bob"));
        assert_eq!(r.port, Some(2222));
        assert_eq!(
            r.identity_files,
            vec![PathBuf::from("/home/u/.ssh/web_key")]
        );
        assert_eq!(r.proxy_jump.as_deref(), Some("jump"));
        assert!(r.identities_only);
    }

    #[test]
    fn unknown_host_defaults() {
        let r = cfg("Host web\n User bob\n").resolve("other");
        assert_eq!(r.hostname, "other");
        assert_eq!(r.user, None);
        assert_eq!(r.port, None);
        assert!(r.identity_files.is_empty());
        assert!(!r.identities_only);
    }

    #[test]
    fn first_match_wins() {
        let c = cfg(
            "Host web\n User first\n Port 1\nHost *\n User second\n Port 2\n HostName %h.lan\n",
        );
        let r = c.resolve("web");
        assert_eq!(r.user.as_deref(), Some("first"));
        assert_eq!(r.port, Some(1));
        assert_eq!(r.hostname, "web.lan");
        // specific after general: general wins
        let c = cfg("Host *\n User gen\nHost web\n User spec\n");
        assert_eq!(c.resolve("web").user.as_deref(), Some("gen"));
    }

    #[test]
    fn global_section_before_host() {
        let c = cfg("User global\nHost web\n User local\n");
        assert_eq!(c.resolve("web").user.as_deref(), Some("global"));
    }

    #[test]
    fn identity_files_accumulate() {
        let c = cfg("Host web\n IdentityFile ~/a\nHost *\n IdentityFile /b\n IdentityFile none\n");
        assert_eq!(
            c.resolve("web").identity_files,
            vec![PathBuf::from("/home/u/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn multiple_patterns_and_negation_in_config() {
        let c = cfg("Host a.* b !a.skip\n User x\n");
        assert_eq!(c.resolve("a.one").user.as_deref(), Some("x"));
        assert_eq!(c.resolve("b").user.as_deref(), Some("x"));
        assert_eq!(c.resolve("a.skip").user, None);
        assert_eq!(c.resolve("c").user, None);
    }

    #[test]
    fn syntax_variants() {
        let c = cfg("# comment\n\nhost = web # trailing\n  user=bob\n  PORT   22\n  Unknown thing\n  IdentityFile \"/path with space/k\"\n");
        let r = c.resolve("web");
        assert_eq!(r.user.as_deref(), Some("bob"));
        assert_eq!(r.port, Some(22));
        assert_eq!(r.identity_files, vec![PathBuf::from("/path with space/k")]);
    }

    #[test]
    fn invalid_port_ignored_and_proxy_none() {
        let c = cfg("Host web\n Port abc\n ProxyJump none\nHost *\n Port 22\n ProxyJump j\n");
        let r = c.resolve("web");
        assert_eq!(r.port, Some(22));
        assert_eq!(r.proxy_jump, None);
    }

    #[test]
    fn match_blocks_skipped() {
        let c = cfg("Match host web\n User m\nHost web\n User h\n");
        assert_eq!(c.resolve("web").user.as_deref(), Some("h"));
    }

    #[test]
    fn include_with_glob_and_depth() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::create_dir_all(home.join(".ssh/conf.d")).unwrap();
        std::fs::write(home.join(".ssh/conf.d/a.conf"), "Host inc\n User fromA\n").unwrap();
        std::fs::write(home.join(".ssh/conf.d/b.conf"), "Host inc\n Port 99\n").unwrap();
        // self-including file must terminate
        std::fs::write(home.join(".ssh/loop"), "Include loop\nHost loop\n User l\n").unwrap();
        let main = home.join(".ssh/config");
        std::fs::write(
            &main,
            "Include conf.d/*.conf\nInclude loop\nHost inc\n User main\nHost *\n Port 1\n",
        )
        .unwrap();
        let mut c = SshConfig::empty();
        c.set_home(home);
        c.parse_text(&std::fs::read_to_string(&main).unwrap(), 0);
        let r = c.resolve("inc");
        assert_eq!(r.user.as_deref(), Some("fromA"));
        assert_eq!(r.port, Some(99));
        assert_eq!(c.resolve("loop").user.as_deref(), Some("l"));
        // Include at top of file restores the global context afterwards
        assert_eq!(c.resolve("zzz").port, Some(1));
    }

    #[test]
    fn load_missing_file_is_empty() {
        let c = SshConfig::load("/nonexistent/oxfer/ssh_config").unwrap();
        assert_eq!(c.resolve("x").hostname, "x");
    }
}
