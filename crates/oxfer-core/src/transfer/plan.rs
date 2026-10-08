use crate::{Entry, EntryKind, Error, RemotePath, Result};

use super::glob;
use super::{JobId, Transfer};

#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub src: Entry,
    pub dst: RemotePath,
}

/// What a transfer is going to do, computed before anything is copied.
#[derive(Debug, Default)]
pub struct Plan {
    /// Destination directories to create, parents first.
    pub dirs: Vec<RemotePath>,
    pub jobs: Vec<Job>,
    /// Source entries that were left out (symlinks, unsafe names).
    pub skipped: Vec<(RemotePath, String)>,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.jobs.iter().map(|j| j.src.size).sum()
    }

    fn push_job(&mut self, src: Entry, dst: RemotePath) {
        let id = self.jobs.len();
        self.jobs.push(Job { id, src, dst });
    }
}

/// Reject names that could escape the destination directory.
fn is_safe_name(name: &str) -> bool {
    !(name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\0'])
        || (cfg!(windows) && name.contains(['\\', ':'])))
}

impl Transfer {
    /// Plan copying each of `sources` (globs allowed) into the existing
    /// directory `dst_dir`, keeping their names.
    pub async fn plan(&self, sources: &[RemotePath], dst_dir: &RemotePath) -> Result<Plan> {
        let dst = self.dst.stat(dst_dir).await?;
        if !dst.is_dir() {
            return Err(Error::Invalid(format!("{dst_dir} is not a directory")));
        }
        let mut plan = Plan::default();
        for pattern in sources {
            for src in glob::expand(&*self.src, pattern).await? {
                let name = src
                    .file_name()
                    .filter(|n| is_safe_name(n))
                    .ok_or_else(|| Error::Invalid(format!("cannot copy {src}")))?;
                let target = dst_dir.join(name);
                self.plan_entry(&mut plan, &src, target).await?;
            }
        }
        Ok(plan)
    }

    /// Plan copying a single `src` to the exact path `dst` (file or directory).
    pub async fn plan_to(&self, src: &RemotePath, dst: &RemotePath) -> Result<Plan> {
        let mut plan = Plan::default();
        self.plan_entry(&mut plan, src, dst.clone()).await?;
        Ok(plan)
    }

    async fn plan_entry(&self, plan: &mut Plan, src: &RemotePath, dst: RemotePath) -> Result<()> {
        let entry = self.src.stat(src).await?;
        match entry.kind {
            EntryKind::File => plan.push_job(entry, dst),
            EntryKind::Dir if self.opts.recursive => self.walk(plan, entry.path, dst).await?,
            EntryKind::Dir => {
                return Err(Error::Invalid(format!(
                    "{src} is a directory (recursive transfer not enabled)"
                )))
            }
            _ => plan
                .skipped
                .push((src.clone(), "not a regular file".into())),
        }
        Ok(())
    }

    async fn walk(
        &self,
        plan: &mut Plan,
        src_root: RemotePath,
        dst_root: RemotePath,
    ) -> Result<()> {
        let mut stack = vec![(src_root, dst_root)];
        while let Some((src_dir, dst_dir)) = stack.pop() {
            plan.dirs.push(dst_dir.clone());
            let mut entries = self.src.list(&src_dir).await?;
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            for e in entries {
                let name = e.name().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                // The backend's idea of the child path must be exactly dir/name,
                // otherwise a hostile server could point us outside the tree.
                if !is_safe_name(&name) || e.path != src_dir.join(&name) {
                    plan.skipped
                        .push((e.path.clone(), "unsafe file name".into()));
                    continue;
                }
                let target = dst_dir.join(&name);
                match e.kind {
                    EntryKind::File => plan.push_job(e, target),
                    EntryKind::Dir => stack.push((e.path, target)),
                    EntryKind::Symlink => plan.skipped.push((e.path, "symlink".into())),
                    EntryKind::Other => plan.skipped.push((e.path, "not a regular file".into())),
                }
            }
        }
        // Parents sort before children, so creating in order is safe.
        plan.dirs.sort();
        Ok(())
    }
}
