# oxfer

A fast, safe SFTP/FTP client written in Rust: command line, interactive shell,
terminal UI and (in progress) a desktop app, all on one transfer engine.

See [plan.md](plan.md) for the roadmap.

## Features

- **SFTP** over a pure-Rust SSH stack (`russh`): password, key file and
  ssh-agent authentication; strict `known_hosts` checking with an
  accept-and-save prompt; a *changed* host key is always refused.
- **Transfer engine**: recursive directories, globs, parallel files, resume of
  partial files, retries with exponential backoff, overwrite policies
  (`always` / `skip` / `newer`), optional SHA-256 verification, Ctrl-C to cancel.
  Downloads and uploads go to `<name>.oxfer-part` first and are renamed when
  complete, so a half-written file is never mistaken for the real one.
- Server-supplied file names are validated, so a hostile server cannot make a
  download write outside the destination directory.

## Usage

```
oxfer ls user@host:/var/log
oxfer get -r user@host:/srv/data ./data --verify -j 8
oxfer put -r ./site user@host:/var/www --overwrite newer
oxfer rm -r user@host:/tmp/old
oxfer shell user@host            # interactive: cd, ls, get, put, tab completion
oxfer completions zsh > _oxfer   # shell completions (bash, zsh, fish, ...)
```

Common options: `-P <port>`, `-i <key>`, `--agent`, `--known-hosts <file>`.
Without `-i`/`--agent`, oxfer tries the ssh-agent, then `~/.ssh/id_*`, then asks
for a password.

## Performance

Loopback comparison against OpenSSH `sftp` (Apple silicon, release build,
`cargo run --release -p oxfer-cli --example bench`; loopback measures client
overhead, not network speed):

| scenario | oxfer | sftp |
|---|---|---|
| upload 256 MiB file | 0.90 s | 0.40 s |
| download 256 MiB file | 0.51 s | 0.37 s |
| upload 1000 × 4 KiB files | 0.42 s | 0.24 s |
| download 1000 × 4 KiB files | 0.19 s | 0.27 s |

Large single-file transfers are currently slower than OpenSSH, which keeps more
requests in flight; closing that gap is future work.

## Layout

- `crates/oxfer-core` – `RemoteFs` trait, `LocalFs`, errors, transfer engine
- `crates/oxfer-sftp` – SFTP backend
- `crates/oxfer-ftp` – FTP/FTPS backend
- `crates/oxfer-cli` – the `oxfer` binary
- `crates/oxfer-gui` / `ui/` – desktop app (Tauri)
- `crates/oxfer-testutil` – throwaway `sshd` for integration tests

## Development

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Integration tests start an unprivileged local `sshd` and skip themselves when
`sshd` or `sftp-server` are not installed.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
