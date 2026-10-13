//! Interactive terminals (an SSH shell on a pty) opened on a session.

use serde::Serialize;

/// Terminal output and lifecycle, emitted to the UI as `terminal-event`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEvent {
    /// Raw bytes from the remote terminal (may split a UTF-8 sequence).
    Output { terminal_id: u64, data: Vec<u8> },
    /// The shell ended or the connection dropped.
    Closed { terminal_id: u64 },
}

/// Receives terminal events (the Tauri layer emits them to the window).
pub trait TerminalSink: Send + Sync {
    fn emit(&self, event: TerminalEvent);
}

impl<F: Fn(TerminalEvent) + Send + Sync> TerminalSink for F {
    fn emit(&self, event: TerminalEvent) {
        self(event)
    }
}
