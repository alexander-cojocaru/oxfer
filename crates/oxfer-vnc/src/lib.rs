//! A small VNC (RFB) client.
//!
//! [`connect`] performs the handshake (no authentication, VNC password or
//! Apple Remote Desktop with user name and password) and returns a
//! [`Connection`]: [`Event`]s describing screen changes arrive on a channel,
//! and the [`Handle`] sends keyboard, pointer and clipboard input.
//!
//! The pixel format is fixed to 32-bit R, G, B, unused, so decoded rectangles
//! are RGBA ready for a canvas.

mod auth;
mod encodings;
mod error;
mod handshake;
mod message;
mod session;

pub use error::{Error, Result};
pub use handshake::ServerInit;
pub use message::{Event, Op};
pub use session::{connect, establish, Connection, Handle};

use std::time::Duration;

#[derive(Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    /// Needed for Apple Remote Desktop (macOS Screen Sharing) authentication.
    pub username: Option<String>,
    pub password: Option<String>,
    /// Limit for connecting and the handshake.
    pub timeout: Duration,
}

impl ConnectOptions {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            password: None,
            timeout: Duration::from_secs(20),
        }
    }
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}
