use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Handle to pause, resume or cancel a running transfer. Cheap to clone.
#[derive(Clone)]
pub struct Control {
    cancel: CancellationToken,
    pause: Arc<watch::Sender<bool>>,
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            pause: Arc::new(watch::channel(false).0),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn pause(&self) {
        self.pause.send_replace(true);
    }

    pub fn resume(&self) {
        self.pause.send_replace(false);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn is_paused(&self) -> bool {
        *self.pause.borrow()
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Returns `Err(Cancelled)` when cancelled; waits while paused.
    pub(crate) async fn checkpoint(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !self.is_paused() {
            return Ok(());
        }
        let mut rx = self.pause.subscribe();
        while *rx.borrow_and_update() {
            tokio::select! {
                _ = self.cancel.cancelled() => return Err(Error::Cancelled),
                r = rx.changed() => if r.is_err() { break },
            }
        }
        Ok(())
    }
}
