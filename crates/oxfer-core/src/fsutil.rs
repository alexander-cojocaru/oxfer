use crate::{EntryKind, RemoteFs, RemotePath, Result};

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
