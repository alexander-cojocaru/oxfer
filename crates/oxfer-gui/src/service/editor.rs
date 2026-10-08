//! Text previews and editing files in the user's text editor.
//!
//! A remote file is downloaded to a private temporary directory and opened
//! there; a watcher uploads it back (as an ordinary transfer, so it shows in
//! the queue) whenever the editor saves it. The copy is removed when the
//! session closes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use oxfer_core::{RemoteFs, RemotePath};
use serde::Serialize;
use tokio::io::AsyncReadExt;

use super::error::{ServiceError, ServiceResult};

/// Bytes read for a preview; longer files are cut off.
pub const PREVIEW_LIMIT: usize = 1024 * 1024;
/// Largest remote file opened in an editor.
pub const EDIT_LIMIT: u64 = 64 * 1024 * 1024;
/// How often an edited copy is checked for changes.
pub(crate) const WATCH_INTERVAL: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    /// `None` for binary files.
    pub content: Option<String>,
    pub binary: bool,
    /// Only the first [`PREVIEW_LIMIT`] bytes are included.
    pub truncated: bool,
    pub size: u64,
}

pub(crate) async fn read_preview(
    fs: &dyn RemoteFs,
    path: &RemotePath,
) -> ServiceResult<FilePreview> {
    let entry = fs.stat(path).await?;
    if entry.is_dir() {
        return Err(ServiceError::invalid(format!("{path} is a directory")));
    }
    let mut buf = Vec::with_capacity((entry.size as usize).min(PREVIEW_LIMIT + 1));
    {
        let reader = fs.open_read(path, 0).await?;
        reader
            .take(PREVIEW_LIMIT as u64 + 1)
            .read_to_end(&mut buf)
            .await?;
    }
    let truncated = buf.len() > PREVIEW_LIMIT;
    buf.truncate(PREVIEW_LIMIT);
    Ok(decode(buf, truncated, entry.size))
}

/// UTF-8 text (invalid sequences replaced), or `binary` if it does not look
/// like text.
pub(crate) fn decode(mut buf: Vec<u8>, truncated: bool, size: u64) -> FilePreview {
    let binary = |size| FilePreview {
        content: None,
        binary: true,
        truncated: false,
        size,
    };
    if buf.iter().take(8192).any(|&b| b == 0) {
        return binary(size);
    }
    if truncated {
        // Do not report a character split by the cut as invalid.
        if let Err(e) = std::str::from_utf8(&buf) {
            if e.error_len().is_none() {
                buf.truncate(e.valid_up_to());
            }
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let replaced = text
        .chars()
        .filter(|&c| c == char::REPLACEMENT_CHARACTER)
        .count();
    if replaced > 8 && replaced * 20 > text.chars().count() {
        return binary(size);
    }
    FilePreview {
        content: Some(text.into_owned()),
        binary: false,
        truncated,
        size,
    }
}

/// Opens a local file in a text editor.
pub type Opener = Arc<dyn Fn(&Path) -> std::io::Result<()> + Send + Sync>;

/// The platform's default text editor.
pub fn system_opener() -> Opener {
    Arc::new(|path: &Path| {
        use std::process::Command;
        #[cfg(target_os = "macos")]
        let status = Command::new("open").arg("-t").arg(path).status();
        #[cfg(windows)]
        let status = Command::new("notepad.exe")
            .arg(path)
            .spawn()
            .map(|_| None::<std::process::ExitStatus>);
        #[cfg(not(any(target_os = "macos", windows)))]
        let status = Command::new("xdg-open").arg(path).status();
        #[cfg(not(windows))]
        let status = status.map(Some);
        match status? {
            Some(s) if !s.success() => Err(std::io::Error::other(format!(
                "could not open an editor ({s})"
            ))),
            _ => Ok(()),
        }
    })
}

/// A file name usable on the local file system.
pub(crate) fn local_name(remote: &RemotePath) -> String {
    let name = remote.file_name().unwrap_or("file");
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    match cleaned.trim_matches(['.', ' ']) {
        "" => "file".into(),
        _ => cleaned,
    }
}

/// Size and modification time, to notice saves.
pub(crate) fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Download `path` from `fs` to `local`.
pub(crate) async fn download(
    fs: &dyn RemoteFs,
    path: &RemotePath,
    local: &PathBuf,
) -> ServiceResult<()> {
    let mut reader = fs.open_read(path, 0).await?;
    let mut file = tokio::fs::File::create(local).await?;
    tokio::io::copy(&mut reader, &mut file).await?;
    file.sync_all().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_text_and_detects_binary() {
        let p = decode(b"hello\nworld".to_vec(), false, 11);
        assert_eq!(p.content.as_deref(), Some("hello\nworld"));
        assert!(!p.binary && !p.truncated);

        assert!(decode(vec![1, 2, 0, 3], false, 4).binary);
        assert!(decode(vec![0xff; 100], false, 100).binary);
        // a few invalid bytes in text (e.g. Latin-1) still preview
        let p = decode(
            b"caf\xe9 au lait, tr\xe8s bien, merci beaucoup".to_vec(),
            false,
            40,
        );
        assert!(!p.binary);
        assert!(p.content.unwrap().starts_with("caf\u{fffd}"));
    }

    #[test]
    fn truncation_does_not_split_characters() {
        let mut buf = "aé".as_bytes().to_vec();
        buf.pop(); // cut in the middle of 'é'
        let p = decode(buf, true, 1000);
        assert_eq!(p.content.as_deref(), Some("a"));
        assert!(p.truncated);
    }

    #[test]
    fn local_names_are_safe() {
        assert_eq!(local_name(&RemotePath::new("/a/notes.txt")), "notes.txt");
        assert_eq!(local_name(&RemotePath::new("/a/b:c?.txt")), "b_c_.txt");
        assert_eq!(local_name(&RemotePath::new("/a/..")), "file");
    }
}
