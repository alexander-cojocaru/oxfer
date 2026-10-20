//! Serializable data transfer objects shared with the frontend (camelCase).

use std::time::UNIX_EPOCH;

use oxfer_core::{Entry, EntryKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKindDto {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDto {
    pub name: String,
    pub path: String,
    pub kind: EntryKindDto,
    pub size: u64,
    /// Milliseconds since the Unix epoch.
    pub modified: Option<i64>,
    pub permissions: Option<u32>,
}

impl From<&Entry> for EntryDto {
    fn from(e: &Entry) -> Self {
        let kind = match e.kind {
            EntryKind::File => EntryKindDto::File,
            EntryKind::Dir => EntryKindDto::Dir,
            EntryKind::Symlink => EntryKindDto::Symlink,
            EntryKind::Other => EntryKindDto::Other,
        };
        let modified = e.modified.and_then(|t| match t.duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).ok(),
            Err(e) => i64::try_from(e.duration().as_millis()).ok().map(|v| -v),
        });
        Self {
            name: e.name().to_string(),
            path: e.path.to_string(),
            kind,
            size: e.size,
            modified,
            permissions: e.permissions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtocolDto {
    Local,
    Sftp,
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
    Vnc,
}

impl From<oxfer_config::Protocol> for ProtocolDto {
    fn from(p: oxfer_config::Protocol) -> Self {
        use oxfer_config::Protocol as P;
        match p {
            P::Sftp => Self::Sftp,
            P::Ftp => Self::Ftp,
            P::FtpsExplicit => Self::FtpsExplicit,
            P::FtpsImplicit => Self::FtpsImplicit,
            P::Vnc => Self::Vnc,
        }
    }
}

impl ProtocolDto {
    pub fn to_config(self) -> Option<oxfer_config::Protocol> {
        use oxfer_config::Protocol as P;
        match self {
            Self::Local => None,
            Self::Sftp => Some(P::Sftp),
            Self::Ftp => Some(P::Ftp),
            Self::FtpsExplicit => Some(P::FtpsExplicit),
            Self::FtpsImplicit => Some(P::FtpsImplicit),
            Self::Vnc => Some(P::Vnc),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub label: String,
    pub protocol: ProtocolDto,
    /// Directory to show first.
    pub home: String,
}
