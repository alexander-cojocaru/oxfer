use std::fmt;

/// A POSIX-style remote path (always `/`-separated, independent of the local OS).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemotePath(String);

impl RemotePath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
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
