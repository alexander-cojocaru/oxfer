use std::collections::HashMap;

use indicatif::{ProgressBar, ProgressStyle};
use oxfer_core::transfer::{Event, Summary};
use tokio::sync::mpsc::UnboundedReceiver;

/// Render engine events as one overall progress bar. Hidden when stderr is
/// not a terminal.
pub async fn render(mut rx: UnboundedReceiver<Event>) {
    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec} eta {eta}  {msg}",
        )
        .expect("valid template")
        .progress_chars("=> "),
    );
    let mut done: HashMap<usize, u64> = HashMap::new();
    let mut names: HashMap<usize, String> = HashMap::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Planned { bytes, .. } => bar.set_length(bytes),
            Event::Started { id, src, .. } => {
                bar.set_message(src.file_name().unwrap_or("").to_string());
                names.insert(id, src.to_string());
            }
            Event::Progress { id, done: d, .. } => {
                let prev = done.insert(id, d).unwrap_or(0);
                bar.inc(d.saturating_sub(prev));
            }
            Event::Retrying { id, attempt, error } => bar.println(format!(
                "retry {attempt} for {}: {error}",
                names.get(&id).map_or("?", |s| s)
            )),
            Event::Skipped { id, reason } => bar.println(format!(
                "skipped {}: {reason}",
                names.get(&id).map_or("?", |s| s)
            )),
            Event::Failed { id, error } => bar.println(format!(
                "FAILED {}: {error}",
                names.get(&id).map_or("?", |s| s)
            )),
            Event::Finished { .. } => {}
            Event::Done { .. } => break,
        }
    }
    bar.finish_and_clear();
}

pub fn print_summary(s: &Summary) {
    eprintln!(
        "{} file(s) transferred ({} bytes), {} skipped, {} failed{}",
        s.completed,
        s.bytes,
        s.skipped.len(),
        s.failed.len(),
        if s.cancelled { ", cancelled" } else { "" }
    );
    for (path, err) in &s.failed {
        eprintln!("  {path}: {err}");
    }
}
