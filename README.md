# oxfer

A fast, safe SFTP/FTP client written in Rust. One transfer engine powers a
command line, an interactive shell, a terminal UI and a desktop app.

The name is *oxide* (Rust) + *transfer*.

- **Fast**: keeps 64 SFTP requests in flight per file and transfers several
  files in parallel (see [Performance](#performance)).
- **Safe**: strict `known_hosts` checking, files appear only when complete, and
  hostile servers cannot write outside your destination directory.
- **Cross-platform**: the CLI builds on macOS, Linux and Windows; the desktop
  app is currently packaged for macOS.

## Features

- **SFTP** over a pure-Rust SSH stack ([`russh`](https://crates.io/crates/russh)):
  password, key file and ssh-agent authentication, jump hosts, and an
  accept-and-save prompt for new host keys. A *changed* host key is always
  refused.
- **FTP and FTPS** as a secondary backend.
- **Transfer engine**: recursive directories, globs, parallel files, resume of
  partial files, retries with exponential backoff, overwrite policies
  (`always` / `skip` / `newer`), optional SHA-256 verification, Ctrl-C to cancel.
  - Transfers go to a `.oxfer-part` file first and are renamed when complete.
    The partial file's name includes a fingerprint of the source's size and
    modification time, so a transfer only resumes a copy of the same version of
    the file.
  - Server-supplied file names are validated before anything is written.
- **Profiles (bookmarks)** in groups, searchable by name, host, user, group or
  directory, and **shared credentials**: save a login once (password in the OS
  keychain, key file, agent or ask-every-time) and reuse it across profiles.
- **Desktop app**: dual-pane browser, transfer queue, SSH terminal, text
  preview of files (F3 or double-click) and editing in your own text editor
  (F4). A remote file is edited as a private local copy; every save is uploaded
  back, and the copy is removed when you disconnect.
- **Remote screens (VNC)**: the desktop app can view and control a remote
  screen, like `vnc://user:password@host` in the macOS Finder. Choose the
  "VNC / Screen Sharing" protocol in a profile or in quick connect. The viewer is
  built in (the [`oxfer-vnc`](crates/oxfer-vnc) crate) and supports no
  authentication, VNC passwords and Apple Remote Desktop logins (the macOS
  account name and password that Screen Sharing asks for), Raw, CopyRect,
  Hextile and ZRLE encodings, keyboard, mouse and wheel, clipboard in both
  directions and fit-to-window scaling. VNC itself is unencrypted apart from
  the login: use it on a trusted network or through a tunnel. Not yet
  supported: SSH tunnelling, VeNCrypt/TLS and the extended clipboard.

## Install

### Desktop app (macOS)

Download `oxfer-<version>-macos-universal.dmg` from the
[latest release](https://github.com/alexander-cojocaru/oxfer/releases/latest)
(Apple silicon and Intel), open it and drag oxfer to Applications.

The app is signed with a self-signed certificate rather than an Apple Developer
ID, so macOS blocks the first launch. Open System Settings → Privacy & Security
and click "Open Anyway", or run:

```
xattr -dr com.apple.quarantine /Applications/oxfer.app
```

After that, "Check for updates" in the status bar downloads and installs new
releases; updates are verified against a public key built into the app.

### Command line

Release archives for the `oxfer` binary (with man page and shell completions)
are attached to each [release](https://github.com/alexander-cojocaru/oxfer/releases).
Each release also includes a generated Homebrew formula, `oxfer.rb`: copy it
into your own tap (`brew tap-new you/tap`, add it under `Formula/`) and run
`brew install you/tap/oxfer`.

From source (Rust toolchain required):

```
git clone https://github.com/alexander-cojocaru/oxfer
cd oxfer
cargo install --path crates/oxfer-cli
```

To build and install the desktop app from source on macOS (needs Node.js and
`cargo install tauri-cli --version "^2" --locked`):

```
scripts/install-macos-app.sh                 # installs to /Applications
scripts/install-macos-app.sh ~/Applications  # or another folder
```

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
for a password. Run `oxfer --help` or `oxfer <command> --help` for everything
else.

## Performance

Loopback comparison against OpenSSH `sftp` (Apple silicon, release build,
`cargo run --release -p oxfer-cli --example bench`). Loopback measures client
overhead, not network speed.

| scenario | oxfer | sftp |
|---|---|---|
| upload 256 MiB file | 0.20 s | 0.34 s |
| download 256 MiB file | 0.16 s | 0.37 s |
| upload 1000 × 4 KiB files | 0.41 s | 0.23 s |
| download 1000 × 4 KiB files | 0.24 s | 0.27 s |

Medians of three runs. oxfer prefers AES-GCM where the CPU has AES instructions.
Many small uploads are still slower than OpenSSH, because each file costs
several round trips (stat, open, close, stat, rename).

## Project layout

- `crates/oxfer-core`: `RemoteFs` trait, `LocalFs`, errors, transfer engine
- `crates/oxfer-sftp`: SFTP backend
- `crates/oxfer-ftp`: FTP/FTPS backend
- `crates/oxfer-config`: profiles, credentials, `ssh_config` parsing
- `crates/oxfer-cli`: the `oxfer` binary (CLI, shell, TUI)
- `crates/oxfer-gui` and `ui/`: desktop app (Tauri 2, Svelte 5)
- `crates/oxfer-testutil`: throwaway `sshd` for integration tests

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md)
for how to build, test and run the project. Changes are listed in
[CHANGELOG.md](CHANGELOG.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
