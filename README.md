# oxfer

A fast, safe SFTP/FTP client written in Rust, with a CLI and a desktop GUI.

Status: early development. See [plan.md](plan.md) for the roadmap.

## Layout

- `crates/oxfer-core` – protocol-agnostic traits, errors, transfer engine
- `crates/oxfer-sftp` – SFTP backend
- `crates/oxfer-ftp` – FTP/FTPS backend
- `crates/oxfer-cli` – command-line client
- `crates/oxfer-gui` – desktop app backend (Tauri)
- `ui/` – desktop app frontend

## Development

```
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
