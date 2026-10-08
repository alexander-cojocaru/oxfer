//! Operations shared by the one-shot commands and the interactive shell.

use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Args, ValueEnum};
use oxfer_core::transfer::{Control, OverwritePolicy, Summary, Transfer, TransferOptions};
use oxfer_core::{Entry, EntryKind, RemoteFs, RemotePath};

use crate::progress;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Overwrite {
    Always,
    Skip,
    Newer,
}

#[derive(Args, Clone, Debug)]
pub struct TransferArgs {
    /// Copy directories recursively
    #[arg(short, long)]
    pub recursive: bool,
    /// What to do when the destination file exists
    #[arg(long, value_enum, default_value_t = Overwrite::Always)]
    pub overwrite: Overwrite,
    /// Start over instead of resuming partial files
    #[arg(long)]
    pub no_resume: bool,
    /// Compare SHA-256 of source and destination after copying
    #[arg(long)]
    pub verify: bool,
    /// Parallel transfers
    #[arg(short = 'j', long, default_value_t = 4)]
    pub jobs: usize,
    /// Retries per file on transient errors
    #[arg(long, default_value_t = 3)]
    pub retries: u32,
}

impl Default for TransferArgs {
    fn default() -> Self {
        Self {
            recursive: false,
            overwrite: Overwrite::Always,
            no_resume: false,
            verify: false,
            jobs: 4,
            retries: 3,
        }
    }
}

impl TransferArgs {
    pub fn options(&self) -> TransferOptions {
        TransferOptions {
            recursive: self.recursive,
            overwrite: match self.overwrite {
                Overwrite::Always => OverwritePolicy::Overwrite,
                Overwrite::Skip => OverwritePolicy::Skip,
                Overwrite::Newer => OverwritePolicy::NewerOnly,
            },
            resume: !self.no_resume,
            verify: self.verify,
            concurrency: self.jobs,
            retries: self.retries,
            ..Default::default()
        }
    }
}

/// Copy `sources` (on `src`) to `dest` (on `dst`) with progress and Ctrl-C
/// handling. If `dest` is an existing directory the sources are copied into
/// it; otherwise a single source is copied to exactly that path.
pub async fn copy(
    src: Arc<dyn RemoteFs>,
    dst: Arc<dyn RemoteFs>,
    sources: &[RemotePath],
    dest: &RemotePath,
    args: &TransferArgs,
) -> Result<Summary> {
    let transfer = Transfer::new(src, dst.clone(), args.options());
    let dest_is_dir = dst.stat(dest).await.map(|e| e.is_dir()).unwrap_or(false);
    let plan = if dest_is_dir {
        transfer.plan(sources, dest).await?
    } else if let [one] = sources {
        transfer.plan_to(one, dest).await?
    } else {
        bail!("{dest} is not a directory");
    };

    let control = Control::new();
    let ctrl_c = {
        let c = control.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                c.cancel();
            }
        })
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let ui = tokio::spawn(progress::render(rx));
    let summary = transfer.run(plan, Some(tx), control).await;
    let _ = ui.await;
    ctrl_c.abort();
    progress::print_summary(&summary);
    Ok(summary)
}

/// `ls -l`-style line.
pub fn format_entry(e: &Entry) -> String {
    let kind = match e.kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink => 'l',
        EntryKind::File => '-',
        EntryKind::Other => '?',
    };
    let perm = e.permissions.map_or("----".into(), |p| format!("{p:04o}"));
    let when = e
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or("-".to_string(), |d| format_time(d.as_secs() as i64));
    format!("{kind}{perm} {:>12} {when:>16} {}", e.size, e.name())
}

/// `YYYY-MM-DD HH:MM` (UTC) from a unix timestamp.
pub fn format_time(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_time;

    #[test]
    fn formats_utc_time() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_time(951_782_400 + 86_399), "2000-02-29 23:59");
    }
}
