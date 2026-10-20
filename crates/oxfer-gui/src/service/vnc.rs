//! Remote screens (VNC). Unlike file sessions these have no file system:
//! the UI gets the screen as binary messages and sends input back.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oxfer_config::{AuthMethod, Protocol};
use oxfer_vnc::{ConnectOptions, Event};
use serde::Serialize;

use super::connect::{ConnectRequest, QuickConnect};
use super::dto::ProtocolDto;
use super::error::{ServiceError, ServiceResult};
use super::profiles::ProfileStore;
use super::prompt::{Prompt, PromptAnswer, Prompter, SecretKind};

const MAX_PASSWORD_ATTEMPTS: usize = 3;

/// Receives the screen messages of one VNC connection (their format is
/// documented in `oxfer_vnc::Event::encode`).
pub trait VncSink: Send + Sync {
    fn send(&self, message: Vec<u8>);
}

impl<F: Fn(Vec<u8>) + Send + Sync> VncSink for F {
    fn send(&self, message: Vec<u8>) {
        self(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncInfo {
    pub id: u64,
    /// The profile name, or `user@host` for a quick connect.
    pub label: String,
    pub width: u16,
    pub height: u16,
    /// The desktop's own name, as reported by the server.
    pub name: String,
}

#[derive(Default)]
pub(crate) struct VncHub {
    next: AtomicU64,
    open: Arc<Mutex<HashMap<u64, oxfer_vnc::Handle>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where to connect and with what.
struct Target {
    label: String,
    host: String,
    port: u16,
    user: Option<String>,
    /// `None`: ask when the server wants one.
    password: Option<String>,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn resolve(profiles: &ProfileStore, req: ConnectRequest) -> ServiceResult<Target> {
    match req {
        ConnectRequest::Profile { name } => {
            let p = profiles.get(&name)?;
            if p.protocol != Protocol::Vnc {
                return Err(ServiceError::invalid(format!(
                    "{name:?} is not a VNC profile"
                )));
            }
            let password = match p.auth {
                AuthMethod::Password => profiles
                    .secrets()
                    .get(&p.secret_key())?
                    .map(|s| s.expose().to_string()),
                _ => None,
            };
            Ok(Target {
                label: p.name.clone(),
                host: p.host.clone(),
                port: p.effective_port(),
                user: non_empty(p.user.as_deref()),
                password,
            })
        }
        ConnectRequest::Quick(q) => resolve_quick(profiles, q),
    }
}

fn resolve_quick(profiles: &ProfileStore, q: QuickConnect) -> ServiceResult<Target> {
    if q.protocol != ProtocolDto::Vnc {
        return Err(ServiceError::invalid("not a VNC connection"));
    }
    let host = q.host.trim().to_string();
    if host.is_empty() {
        return Err(ServiceError::invalid("host is empty"));
    }
    let (user, password) = match non_empty(q.credential.as_deref()) {
        Some(name) => {
            let c = profiles.credential(&name)?;
            let password = match c.auth {
                AuthMethod::Password => profiles
                    .secrets()
                    .get(&c.secret_key())?
                    .map(|s| s.expose().to_string()),
                _ => None,
            };
            (non_empty(c.user.as_deref()), password)
        }
        None => (
            non_empty(q.user.as_deref()),
            q.password.map(|p| p.0).filter(|p| !p.is_empty()),
        ),
    };
    Ok(Target {
        label: match &user {
            Some(u) => format!("{u}@{host}"),
            None => host.clone(),
        },
        port: q.port.unwrap_or_else(|| Protocol::Vnc.default_port()),
        host,
        user,
        password,
    })
}

fn map_error(e: oxfer_vnc::Error) -> ServiceError {
    use oxfer_vnc::Error as E;
    let code = match &e {
        E::Io(_) | E::Timeout => "connection",
        E::Protocol(_) => "protocol",
        E::AuthFailed(_) | E::PasswordRequired | E::UsernameRequired => "auth",
        E::Unsupported(_) => "unsupported",
    };
    ServiceError::new(code, e.to_string())
}

impl VncHub {
    pub(crate) async fn open(
        &self,
        profiles: &ProfileStore,
        req: ConnectRequest,
        prompter: Arc<dyn Prompter>,
        sink: Arc<dyn VncSink>,
    ) -> ServiceResult<VncInfo> {
        let target = resolve(profiles, req)?;
        let mut password = target.password.clone();
        let mut asked = 0;
        let conn = loop {
            let mut o = ConnectOptions::new(target.host.clone(), target.port);
            o.username = target.user.clone();
            o.password = password.clone();
            match oxfer_vnc::connect(o).await {
                Ok(c) => break c,
                Err(e @ (oxfer_vnc::Error::PasswordRequired | oxfer_vnc::Error::AuthFailed(_)))
                    if asked < MAX_PASSWORD_ATTEMPTS =>
                {
                    tracing::debug!("VNC login needs a password: {e}");
                    let answer = prompter
                        .ask(Prompt::Password {
                            user: target.user.clone().unwrap_or_default(),
                            host: target.host.clone(),
                            secret: SecretKind::Password,
                            retry: password.is_some(),
                        })
                        .await;
                    match answer {
                        PromptAnswer::Password { value } => password = Some(value.0),
                        _ => return Err(ServiceError::cancelled()),
                    }
                    asked += 1;
                }
                Err(oxfer_vnc::Error::UsernameRequired) => {
                    return Err(ServiceError::new(
                        "auth",
                        "this server needs a user name (macOS Screen Sharing signs in with a \
                         macOS account): add one to the profile",
                    ))
                }
                Err(e) => return Err(map_error(e)),
            }
        };

        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let info = VncInfo {
            id,
            label: target.label,
            width: conn.width,
            height: conn.height,
            name: conn.name,
        };
        lock(&self.open).insert(id, conn.handle);
        let open = self.open.clone();
        let mut events = conn.events;
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                let closed = matches!(ev, Event::Closed(_));
                sink.send(ev.encode());
                if closed {
                    break;
                }
            }
            if let Some(handle) = lock(&open).remove(&id) {
                handle.close();
            }
        });
        Ok(info)
    }

    fn handle(&self, id: u64) -> ServiceResult<oxfer_vnc::Handle> {
        lock(&self.open)
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::new("unknownVnc", format!("no remote screen #{id}")))
    }

    pub(crate) fn key(&self, id: u64, keysym: u32, down: bool) -> ServiceResult<()> {
        self.handle(id)?.key(keysym, down);
        Ok(())
    }

    pub(crate) fn pointer(&self, id: u64, x: u16, y: u16, buttons: u8) -> ServiceResult<()> {
        self.handle(id)?.pointer(x, y, buttons);
        Ok(())
    }

    pub(crate) fn clipboard(&self, id: u64, text: String) -> ServiceResult<()> {
        self.handle(id)?.clipboard(text);
        Ok(())
    }

    pub(crate) fn close(&self, id: u64) -> ServiceResult<()> {
        let handle = lock(&self.open)
            .remove(&id)
            .ok_or_else(|| ServiceError::new("unknownVnc", format!("no remote screen #{id}")))?;
        handle.close();
        Ok(())
    }
}
