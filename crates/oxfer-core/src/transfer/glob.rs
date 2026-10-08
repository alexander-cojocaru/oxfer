use globset::Glob;

use crate::{Error, RemoteFs, RemotePath, Result};

pub fn has_glob(s: &str) -> bool {
    s.contains(['*', '?', '[', '{'])
}

/// Expand a glob in the last path component (`/data/*.csv`). Paths without
/// metacharacters are returned unchanged.
pub async fn expand(fs: &dyn RemoteFs, pattern: &RemotePath) -> Result<Vec<RemotePath>> {
    let Some(name) = pattern.file_name().filter(|n| has_glob(n)) else {
        return Ok(vec![pattern.clone()]);
    };
    let parent = pattern.parent().unwrap_or_else(|| RemotePath::new("."));
    let matcher = Glob::new(name)
        .map_err(|e| Error::Invalid(format!("bad glob {name:?}: {e}")))?
        .compile_matcher();
    let mut hits: Vec<_> = fs
        .list(&parent)
        .await?
        .into_iter()
        .filter(|e| matcher.is_match(e.name()))
        .map(|e| e.path)
        .collect();
    if hits.is_empty() {
        return Err(Error::NotFound(pattern.clone()));
    }
    hits.sort();
    Ok(hits)
}
