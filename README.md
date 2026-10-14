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
  Downloads and uploads go to a `.oxfer-part` file first and are renamed when
  complete, so a half-written file is never mistaken for the real one. The
  partial file's name includes a fingerprint of the source's size and
  modification time, so a transfer only resumes a partial copy of the same
  version of the file.
- Server-supplied file names are validated, so a hostile server cannot make a
  download write outside the destination directory.
- **Profiles (bookmarks)** in groups, searchable by name, host, user, group or
  directory, and **shared credentials**: save a user and login (password in the
  OS keychain, key file, agent or ask-every-time) once and use it for several
  profiles or quick connects.
- **Desktop app**: dual-pane browser, transfer queue, text preview of files
  (F3 or double-click) and editing in your text editor (F4). A remote file is
  edited as a private local copy; every save is uploaded back as a transfer,
  and the copy is removed when you disconnect.

## Usage

```
oxfer ls user@host:/var/log
oxfer get -r user@host:/srv/data ./data --verify -j 8
oxfer put -r ./site user@host:/var/www --overwrite newer
oxfer rm -r user@host:/tmp/old
oxfer shell user@host            # interactive: cd, ls, get, put, tab completion
oxfer completions zsh > _oxfer   # shell completions (bash, zsh, fish, ...)

oxfer credential add deploy --user deployer          # asks for the password once
oxfer profile add web web.example.com --group Work --credential deploy
oxfer profile list                                    # by group; `profile list acme` searches
oxfer ls @web:/var/www
```

Common options: `-P <port>`, `-i <key>`, `--agent`, `--known-hosts <file>`.
Without `-i`/`--agent`, oxfer tries the ssh-agent, then `~/.ssh/id_*`, then asks
for a password.

## Install

Tagged releases publish binaries (with man page and shell completions) and a
generated Homebrew formula `oxfer.rb` as release assets. Copy the formula into
your own tap (`brew tap-new you/tap`, add `oxfer.rb` under `Formula/`), then
`brew install you/tap/oxfer`. From source: `cargo install --path crates/oxfer-cli`.

Desktop app on macOS: download `oxfer-<version>-macos-universal.dmg` from the
release (Apple silicon and Intel), open it and drag oxfer to Applications. The
app is signed with a self-signed certificate rather than an Apple Developer ID,
so macOS blocks the first launch: open System Settings → Privacy & Security and
click "Open Anyway" (or run `xattr -dr com.apple.quarantine /Applications/oxfer.app`).
Updates are signed with the same certificate, so this is needed only once per
download and keychain permissions carry over.

Or build it from this checkout (needs Node.js and
`cargo install tauri-cli --version "^2" --locked`):

```
scripts/install-macos-app.sh                 # build and install to /Applications
scripts/install-macos-app.sh ~/Applications  # or another folder
```

Run it again after `git pull` to update; it quits a running oxfer first.

The app is signed with a stable identity so macOS keeps keychain and folder
permissions across updates: on first run the script creates a self-signed
certificate "oxfer local signing" in your login keychain (set
`OXFER_SIGN_IDENTITY` to use one of your own). Answer "Always Allow" to
keychain prompts once and they stay answered.

## Performance

Loopback comparison against OpenSSH `sftp` (Apple silicon, release build,
`cargo run --release -p oxfer-cli --example bench`; loopback measures client
overhead, not network speed):

| scenario | oxfer | sftp |
|---|---|---|
| upload 256 MiB file | 0.20 s | 0.34 s |
| download 256 MiB file | 0.16 s | 0.37 s |
| upload 1000 × 4 KiB files | 0.41 s | 0.23 s |
| download 1000 × 4 KiB files | 0.24 s | 0.27 s |

Medians of three runs; the large upload occasionally takes up to 0.5 s.
oxfer prefers AES-GCM where the CPU has AES instructions and keeps 64 SFTP
requests in flight per file. Many small uploads are still slower than OpenSSH,
because each file costs several round trips (stat, open, close, stat, rename).

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

### Desktop GUI

The GUI is a Tauri 2 app (`crates/oxfer-gui`) with a Svelte 5 + TypeScript
frontend (`ui/`). It is not a default workspace member because it needs the
system webview (on Linux: `libwebkit2gtk-4.1-dev libsoup-3.0-dev
libayatana-appindicator3-dev librsvg2-dev`).

```
cd ui && npm ci && npm test && npm run build   # builds ui/dist
cargo run -p oxfer-gui                          # embeds ui/dist
cargo test -p oxfer-gui                         # headless service tests
```

All session, file and transfer logic lives in `oxfer_gui::service` without
Tauri types; `main.rs` only maps commands and events. Icons are generated by
`python3 crates/oxfer-gui/icons/generate.py`.

### Releasing

Pushing a `v*` tag builds the CLI archives, the Homebrew formula and the macOS
`.dmg` (`scripts/package-macos-dmg.sh` signs the app and builds the image).
The `.dmg` job signs with the "oxfer local signing" certificate, so releases and
local `install-macos-app.sh` builds share one identity. Set it up once:

1. Keychain Access → login → My Certificates → right-click "oxfer local
   signing" → Export… → save as `oxfer-signing.p12` with a password.
2. Store it as repository secrets, then delete the file:

   ```
   base64 -i oxfer-signing.p12 | gh secret set MACOS_SIGNING_P12
   gh secret set MACOS_SIGNING_P12_PASSWORD   # the export password
   rm oxfer-signing.p12
   ```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
