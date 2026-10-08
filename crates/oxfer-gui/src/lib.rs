//! oxfer desktop GUI backend.
//!
//! [`service`] contains all session, file and transfer logic without any
//! Tauri types so it can be tested headlessly; `main.rs` is a thin Tauri layer.

pub mod service;
