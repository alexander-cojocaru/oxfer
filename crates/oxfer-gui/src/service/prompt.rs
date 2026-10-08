//! Interactive questions (host keys, passwords, conflicts) asked through the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::dto::EntryDto;
use super::error::{ServiceError, ServiceResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    Password,
    Passphrase,
}

/// A question for the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Prompt {
    /// The SSH host key is not in `known_hosts` yet.
    HostKey {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
    },
    /// A password or key passphrase is needed.
    Password {
        user: String,
        host: String,
        secret: SecretKind,
        /// Set when a previous attempt was rejected.
        retry: bool,
    },
    /// A yes/no question, e.g. connecting over plain FTP.
    Confirm {
        title: String,
        message: String,
        /// Whether accepting is security-relevant (UI shows a warning style).
        danger: bool,
    },
    /// A transfer destination already exists (overwrite policy `ask`).
    Conflict {
        transfer_id: u64,
        src: EntryDto,
        dst: EntryDto,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKeyChoice {
    AcceptAndSave,
    AcceptOnce,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictChoice {
    Overwrite,
    Skip,
}

/// A string that never shows up in `Debug` output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(pub String);

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"..\"")
    }
}

/// The user's answer to a [`Prompt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PromptAnswer {
    HostKey {
        decision: HostKeyChoice,
    },
    Password {
        value: SecretString,
    },
    Confirm {
        ok: bool,
    },
    Conflict {
        action: ConflictChoice,
        apply_to_all: bool,
    },
    /// Dialog dismissed.
    Cancel,
}

/// Asks the user something and waits for the answer.
#[async_trait]
pub trait Prompter: Send + Sync {
    async fn ask(&self, prompt: Prompt) -> PromptAnswer;
}

/// Run an async prompt from synchronous code that is executing on a tokio
/// worker thread (the SSH host-key callback is synchronous). Requires the
/// multi-threaded runtime; anywhere else the prompt counts as dismissed.
pub(crate) fn ask_blocking(prompter: &Arc<dyn Prompter>, prompt: Prompt) -> PromptAnswer {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| h.block_on(prompter.ask(prompt)))
        }
        _ => {
            tracing::warn!("cannot prompt outside a multi-threaded runtime; rejecting");
            PromptAnswer::Cancel
        }
    }
}

/// Notification sent to the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PromptEvent {
    Open {
        id: u64,
        prompt: Prompt,
    },
    /// The question is no longer relevant (answered, or its operation ended).
    Closed {
        id: u64,
    },
}

type Emit = Box<dyn Fn(PromptEvent) + Send + Sync>;

/// [`Prompter`] that forwards questions through an `emit` callback (the Tauri
/// layer turns them into `prompt` events) and waits for [`answer`](Self::answer).
pub struct PromptBroker {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<PromptAnswer>>>,
    emit: Emit,
}

impl PromptBroker {
    pub fn new(emit: impl Fn(PromptEvent) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            emit: Box::new(emit),
        })
    }

    /// Deliver the user's answer to the prompt `id`.
    pub fn answer(&self, id: u64, answer: PromptAnswer) -> ServiceResult<()> {
        let tx = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
            .ok_or_else(|| ServiceError::new("unknownPrompt", format!("no open prompt #{id}")))?;
        // The asking side may have gone away meanwhile; that is fine.
        let _ = tx.send(answer);
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

struct Pending<'a> {
    broker: &'a PromptBroker,
    id: u64,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.broker
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        (self.broker.emit)(PromptEvent::Closed { id: self.id });
    }
}

#[async_trait]
impl Prompter for PromptBroker {
    async fn ask(&self, prompt: Prompt) -> PromptAnswer {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let _guard = Pending { broker: self, id };
        (self.emit)(PromptEvent::Open { id, prompt });
        rx.await.unwrap_or(PromptAnswer::Cancel)
    }
}

/// Prompter answering from a fixed script; records every question. For tests
/// and headless use.
#[derive(Default)]
pub struct ScriptedPrompter {
    answers: Mutex<std::collections::VecDeque<PromptAnswer>>,
    asked: Mutex<Vec<Prompt>>,
}

impl ScriptedPrompter {
    pub fn new(answers: impl IntoIterator<Item = PromptAnswer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::default(),
        })
    }

    pub fn asked(&self) -> Vec<Prompt> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
impl Prompter for ScriptedPrompter {
    async fn ask(&self, prompt: Prompt) -> PromptAnswer {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(prompt);
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or(PromptAnswer::Cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broker_roundtrip_and_close() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let broker = PromptBroker::new(move |e| ev.lock().unwrap().push(e));
        let b = broker.clone();
        let task = tokio::spawn(async move {
            b.ask(Prompt::Confirm {
                title: "t".into(),
                message: "m".into(),
                danger: false,
            })
            .await
        });
        // wait for the prompt to be opened
        let id = loop {
            if let Some(PromptEvent::Open { id, .. }) = events.lock().unwrap().first() {
                break *id;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(broker.pending(), 1);
        broker
            .answer(id, PromptAnswer::Confirm { ok: true })
            .unwrap();
        assert_eq!(task.await.unwrap(), PromptAnswer::Confirm { ok: true });
        assert_eq!(broker.pending(), 0);
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(PromptEvent::Closed { id: c }) if *c == id
        ));
        assert_eq!(
            broker.answer(id, PromptAnswer::Cancel).unwrap_err().code,
            "unknownPrompt"
        );
    }

    #[tokio::test]
    async fn dropped_question_is_closed() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let broker = PromptBroker::new(move |e| ev.lock().unwrap().push(e));
        let fut = broker.ask(Prompt::Confirm {
            title: "t".into(),
            message: "m".into(),
            danger: false,
        });
        let r = tokio::time::timeout(std::time::Duration::from_millis(20), fut).await;
        assert!(r.is_err());
        assert_eq!(broker.pending(), 0);
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(PromptEvent::Closed { .. })
        ));
    }

    #[test]
    fn json_shape() {
        let p = Prompt::HostKey {
            host: "h".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:x".into(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "hostKey");
        assert_eq!(v["fingerprint"], "SHA256:x");
        let a: PromptAnswer =
            serde_json::from_str(r#"{"kind":"conflict","action":"overwrite","applyToAll":true}"#)
                .unwrap();
        assert_eq!(
            a,
            PromptAnswer::Conflict {
                action: ConflictChoice::Overwrite,
                apply_to_all: true
            }
        );
        let a: PromptAnswer =
            serde_json::from_str(r#"{"kind":"password","value":"hunter2"}"#).unwrap();
        assert!(!format!("{a:?}").contains("hunter2"));
        let a: PromptAnswer =
            serde_json::from_str(r#"{"kind":"hostKey","decision":"acceptAndSave"}"#).unwrap();
        assert_eq!(
            a,
            PromptAnswer::HostKey {
                decision: HostKeyChoice::AcceptAndSave
            }
        );
    }
}
