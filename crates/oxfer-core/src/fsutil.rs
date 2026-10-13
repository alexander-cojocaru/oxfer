use crate::{EntryKind, Error, RemoteFs, RemotePath, Result};

/// Remove a file, or a directory with everything below it. Symlinks are
/// removed, never followed.
pub async fn remove_all(fs: &dyn RemoteFs, path: &RemotePath) -> Result<()> {
    // Post-order walk with an explicit stack: (path, children already queued).
    let mut stack = vec![(path.clone(), false)];
    while let Some((p, expanded)) = stack.pop() {
        let entry = fs.lstat(&p).await?;
        if entry.kind != EntryKind::Dir || expanded {
            fs.remove(&p).await?;
            continue;
        }
        stack.push((p.clone(), true));
        for child in fs.list(&p).await? {
            // Guard against backends reporting paths outside `p`.
            if child.path == p.join(child.name()) {
                stack.push((child.path, false));
            }
        }
    }
    Ok(())
}

/// Create `path` and any missing parent directories. Succeeds if `path` is
/// already a directory; fails if it, or a parent, exists as a non-directory.
pub async fn mkdir_all(fs: &dyn RemoteFs, path: &RemotePath) -> Result<()> {
    // Walk up to the nearest existing ancestor, then create downwards.
    let mut missing = Vec::new();
    let mut cur = path.normalize();
    loop {
        match fs.stat(&cur).await {
            Ok(e) if e.is_dir() => break,
            Ok(_) => return Err(Error::AlreadyExists(cur)),
            Err(Error::NotFound(_)) => {
                let Some(parent) = cur.parent() else { break };
                missing.push(cur);
                cur = parent;
            }
            Err(e) => return Err(e),
        }
    }
    for dir in missing.into_iter().rev() {
        match fs.mkdir(&dir).await {
            // lost a race with another creator
            Err(Error::AlreadyExists(_)) => {}
            r => r?,
        }
    }
    Ok(())
}
