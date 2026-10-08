# oxfer — Rust SFTP/FTP Client

**Name:** `oxfer` = *oxide* (Rust) + *transfer*. Short, easy to type, binary name `oxfer`.
Alternatives: `ferrox`, `crabdrop`, `shipit`.

## Goals
- Fast, safe, cross-platform (macOS/Linux/Windows) file transfer client.
- Protocols: SFTP (primary), FTP/FTPS (secondary).
- Usable as a desktop GUI app, a CLI/TUI, and a reusable library crate.

## Non-goals (for now)
- Cloud storage backends (S3, etc.), SCP/rsync protocol, server implementation.

## Architecture
Cargo workspace:

```
oxfer/
├── crates/
│   ├── oxfer-core/     # protocol-agnostic traits, transfer engine, errors
│   ├── oxfer-sftp/     # SFTP backend
│   ├── oxfer-ftp/      # FTP/FTPS backend
│   ├── oxfer-cli/      # binary: CLI + interactive shell (+ later TUI)
│   └── oxfer-gui/      # Tauri backend: exposes core as commands/events to the UI
├── ui/                 # frontend (TypeScript + web UI)
├── plan.md
└── README.md
```

Core abstraction (async):

```rust
#[async_trait]
trait RemoteFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<Entry>>;
    async fn stat(&self, path: &RemotePath) -> Result<Entry>;
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<impl AsyncRead>;
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<impl AsyncWrite>;
    async fn mkdir(&self, path: &RemotePath) -> Result<()>;
    async fn remove(&self, path: &RemotePath) -> Result<()>;
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()>;
}
```

Transfer engine sits on top: queue, concurrency, progress events, resume, retries.

## Candidate dependencies
| Concern | Crate |
|---|---|
| Async runtime | `tokio` |
| SFTP | `russh` + `russh-sftp` (pure Rust) — alt: `ssh2` (libssh2 binding) |
| FTP/FTPS | `suppaftp` (with `tokio-rustls`) |
| CLI parsing | `clap` |
| Interactive shell | `rustyline` |
| Desktop GUI shell | `tauri` (Rust backend + system webview) |
| Frontend | TypeScript + Svelte or React, Vite, virtualized list for large directories |
| TUI (optional) | `ratatui` + `crossterm` |
| Progress | `indicatif` |
| Config | `serde` + `toml`, `directories` |
| Errors / logging | `thiserror`, `anyhow`, `tracing` |
| Secrets | `keyring` |
| Testing | `testcontainers` (OpenSSH + vsftpd/pure-ftpd), `assert_cmd` |

**Early decision:** `russh` (pure Rust, async, easy to build) vs `ssh2` (mature, C dependency). Prototype with `russh` first.

## Status

| Milestone | State |
|---|---|
| M0 Scaffolding | done |
| M1 SFTP basics | done (password / key / agent, known_hosts TOFU, integration tests vs real sshd) |
| M2 Interactive shell | done |
| M3 Transfer engine | done |
| M4 FTP / FTPS | done (plain FTP and FTPS explicit+implicit tested against mock servers, incl. certificate verification; also run by hand against real vsftpd (plain FTP) and pyftpdlib (plain + explicit FTPS, 3 MB roundtrip, mkdir/mv/rm, default cert verification rejects self-signed); implicit FTPS and other servers untested. Real vsftpd's TLS mode crashed in the arm64 container, so TLS there is untested) |
| M5 Config & profiles | done (incl. ProxyJump chains; profile names as jump hosts unsupported) |
| M6 Desktop GUI | done (Tauri 2 + Svelte; builds and starts on macOS; on Linux (Ubuntu 24.04 arm64 container) fmt, clippy incl. the GUI, and the full test suite against a real sshd pass; the GUI also builds and runs for 20 s under Xvfb without panicking, but nobody has clicked through it on Linux) |
| M7 TUI | done (unit/render tests plus a pseudo-terminal test driving a real session) |
| M8 Polish & release | done: completions, man page, release workflow (binaries + generated Homebrew formula), benchmarks, property tests. The release workflow itself has not been run (needs a pushed tag) |

## Milestones

### M0 — Scaffolding
- Cargo workspace, CI (fmt, clippy, test) via GitHub Actions, license, README.
- `tracing` setup, error types, `RemoteFs` trait.

### M1 — SFTP basics (MVP)
- Connect with password and SSH key (incl. passphrase, ssh-agent).
- Host key verification against `~/.ssh/known_hosts` (trust-on-first-use prompt).
- `ls`, `stat`, `get`, `put`, `mkdir`, `rm`, `mv`.
- One-shot CLI: `oxfer get user@host:/path ./local`.

### M2 — Interactive shell
- REPL with `cd`, `lcd`, `pwd`, `ls`, `get`, `put`, tab-completion, history.
- Progress bars with speed/ETA.

### M3 — Transfer engine
- Recursive directory transfers, glob patterns.
- Parallel transfers, resume of partial files, retry with backoff.
- Optional checksum verification after transfer.
- Overwrite policies: ask / skip / overwrite / newer-only.

### M4 — FTP / FTPS
- Implement `RemoteFs` for FTP (passive mode, explicit/implicit TLS).
- Handle server quirks (MLSD vs LIST parsing, UTF-8, EPSV).

### M5 — Config & profiles
- `~/.config/oxfer/config.toml` with named profiles/bookmarks.
- Read `~/.ssh/config` (Host, IdentityFile, ProxyJump).
- Credentials stored in OS keychain.

### M6 — Frontend UI (desktop GUI)
- Dual-pane file browser (local | remote) with drag & drop between panes and from the OS file manager.
- Connection manager: saved profiles, quick connect, host-key trust dialogs, password/key prompts.
- Transfer queue panel: progress, speed/ETA, pause/resume/cancel, retry failed.
- File operations: rename, delete, mkdir, properties/permissions, context menus.
- Remote search/filter, sortable columns, breadcrumb path bar, keyboard shortcuts.
- Light/dark theme following the OS, i18n-ready strings.
- Tauri commands wrap `RemoteFs` and the transfer engine; progress and queue updates are pushed to the UI as events.

### M7 — TUI (optional)
- Dual-pane terminal UI with `ratatui`, sharing the same core.

### M8 — Polish & release
- Cross-platform builds (cargo-dist), Homebrew formula, `cargo install`.
- Docs, man page, shell completions.
- Benchmarks vs `sftp`/`lftp`; fuzz path/listing parsers.

## Testing strategy
- Unit tests: path handling, listing parsers, transfer state machine.
- Integration: dockerized OpenSSH + FTP servers via `testcontainers`.
- CLI tests with `assert_cmd`.
- UI: component tests (Vitest) and end-to-end tests of the Tauri app (WebDriver / Playwright) against a dockerized SFTP server.
- Property/fuzz tests for FTP LIST parsing.

## Security considerations
- Strict host key checking by default; never silently accept changed keys.
- No passwords on the command line or in logs; zeroize secrets in memory.
- FTPS cert validation on by default; warn loudly on plain FTP.
- Sanitize remote filenames to prevent path traversal on download (`../`).

## Open questions
- Library-first or CLI-first release? Decided: CLI-first; crates stay unpublished for now.
- GUI framework: Tauri (recommended: reuses Rust core, small binaries, web UI skills) vs a pure-Rust UI (`egui`, `iced`, `slint`) vs a local web server UI?
- Frontend framework: Svelte (lean) or React (larger ecosystem)?
- Minimum supported Rust version policy?
- Windows support for ssh-agent (named pipe) in M1 or later?
- License: MIT/Apache-2.0 dual (Rust convention)?
