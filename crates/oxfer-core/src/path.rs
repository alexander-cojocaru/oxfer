use std::fmt;

/// A POSIX-style remote path (always `/`-separated, independent of the local OS).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemotePath(String);

impl RemotePath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// A path on the local file system. Windows separators become `/` (which
    /// the Windows APIs accept) so the rest of the code can treat every path
    /// as slash-separated; the `\\?\` prefix of canonicalised paths is dropped.
    pub fn from_local(path: &str) -> Self {
        if cfg!(windows) {
            let p = path.strip_prefix(r"\\?\").unwrap_or(path);
            Self(p.replace('\\', "/"))
        } else {
            Self(path.to_string())
        }
    }

    pub fn root() -> Self {
        Self("/".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_absolute(&self) -> bool {
        self.0.starts_with('/')
    }

    /// Append `child`. An absolute `child` replaces the path, like `Path::join`.
    pub fn join(&self, child: &str) -> Self {
        if child.starts_with('/') {
            return Self::new(child);
        }
        let base = self.0.trim_end_matches('/');
        if base.is_empty() && !self.is_absolute() {
            Self::new(child)
        } else {
            Self(format!("{base}/{child}"))
        }
    }

    /// Resolve `.` and `..` lexically (no symlink resolution). `..` above the
    /// root of an absolute path stays at the root; above a relative path it is kept.
    pub fn normalize(&self) -> Self {
        let mut parts: Vec<&str> = Vec::new();
        for comp in self.0.split('/') {
            match comp {
                "" | "." => {}
                ".." => match parts.last() {
                    Some(&"..") | None if !self.is_absolute() => parts.push(".."),
                    Some(_) => {
                        parts.pop();
                    }
                    None => {}
                },
                c => parts.push(c),
            }
        }
        let joined = parts.join("/");
        match (self.is_absolute(), joined.is_empty()) {
            (true, _) => Self(format!("/{joined}")),
            (false, true) => Self(".".into()),
            (false, false) => Self(joined),
        }
    }

    pub fn parent(&self) -> Option<Self> {
        let trimmed = self.0.trim_end_matches('/');
        let idx = trimmed.rfind('/')?;
        Some(if idx == 0 {
            Self::root()
        } else {
            Self::new(&trimmed[..idx])
        })
    }

    pub fn file_name(&self) -> Option<&str> {
        let trimmed = self.0.trim_end_matches('/');
        let name = trimmed.rsplit('/').next()?;
        (!name.is_empty()).then_some(name)
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for RemotePath {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join() {
        assert_eq!(RemotePath::root().join("a").as_str(), "/a");
        assert_eq!(RemotePath::new("/a/").join("b").as_str(), "/a/b");
        assert_eq!(RemotePath::new("a").join("b").as_str(), "a/b");
        assert_eq!(RemotePath::new("/a").join("/b").as_str(), "/b");
    }

    #[test]
    fn normalize() {
        let n = |s: &str| RemotePath::new(s).normalize().to_string();
        assert_eq!(n("/a/./b/../c"), "/a/c");
        assert_eq!(n("/.."), "/");
        assert_eq!(n("/a//b/"), "/a/b");
        assert_eq!(n("a/../.."), "..");
        assert_eq!(n("a/.."), ".");
        assert_eq!(n("../x"), "../x");
    }

    #[test]
    fn parent() {
        assert_eq!(RemotePath::new("/a/b").parent(), Some("/a".into()));
        assert_eq!(RemotePath::new("/a").parent(), Some(RemotePath::root()));
        assert_eq!(RemotePath::root().parent(), None);
        assert_eq!(RemotePath::new("a").parent(), None);
    }

    #[test]
    fn file_name() {
        assert_eq!(RemotePath::new("/a/b.txt").file_name(), Some("b.txt"));
        assert_eq!(RemotePath::new("/a/b/").file_name(), Some("b"));
        assert_eq!(RemotePath::root().file_name(), None);
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn normalize_is_idempotent_and_has_no_dot_segments(s in "[a-z./]{0,30}") {
            let n = RemotePath::new(s).normalize();
            prop_assert_eq!(n.normalize(), n.clone());
            let abs = n.is_absolute();
            for comp in n.as_str().split('/') {
                prop_assert!(comp != ".." || !abs, "absolute path kept '..': {}", n);
                prop_assert!(comp != "." || n.as_str() == ".", "stray '.': {}", n);
            }
        }

        #[test]
        fn absolute_normalized_paths_never_escape_root(s in "/[a-z./]{0,30}") {
            let n = RemotePath::new(s).normalize();
            prop_assert!(n.is_absolute());
            prop_assert!(!n.as_str().split('/').any(|c| c == ".."));
        }

        #[test]
        fn join_then_file_name_roundtrips(base in "/[a-z]{1,5}(/[a-z]{1,5}){0,3}", name in "[a-z0-9_.-]{1,10}") {
            prop_assume!(name != "." && name != "..");
            let p = RemotePath::new(base).join(&name);
            prop_assert_eq!(p.file_name(), Some(name.as_str()));
        }
    }
}
