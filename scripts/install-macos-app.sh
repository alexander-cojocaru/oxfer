#!/usr/bin/env bash
# Build the desktop app (oxfer.app) from this checkout and install it, so it
# shows up in Launchpad / Spotlight like any other application.
#
#   scripts/install-macos-app.sh                 # installs to /Applications
#   scripts/install-macos-app.sh ~/Applications  # or anywhere else
#   scripts/install-macos-app.sh --no-install    # only build the bundle
#
# Needs Node.js (npm) and the Tauri CLI (`cargo install tauri-cli --version "^2" --locked`).
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "this script builds a macOS .app; on other systems use \`cargo tauri build\` in crates/oxfer-gui" >&2
    exit 1
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="/Applications"
install=1
for arg in "$@"; do
    case "$arg" in
        --no-install) install=0 ;;
        -h|--help) sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) dest="$arg" ;;
    esac
done

# rustup from Homebrew does not put `cargo` on PATH by itself.
if ! command -v cargo >/dev/null && command -v rustup >/dev/null; then
    PATH="$(dirname "$(rustup which cargo)"):$PATH"
fi
PATH="$HOME/.cargo/bin:$PATH"
for tool in cargo npm cargo-tauri; do
    if ! command -v "$tool" >/dev/null; then
        case "$tool" in
            cargo-tauri) hint='cargo install tauri-cli --version "^2" --locked' ;;
            npm) hint='brew install node' ;;
            *) hint='install Rust via rustup' ;;
        esac
        echo "missing $tool ($hint)" >&2
        exit 1
    fi
done

echo "==> building the frontend"
cd "$root/ui"
[[ -d node_modules ]] || npm ci
npm run build

echo "==> building oxfer.app"
cd "$root/crates/oxfer-gui"
cargo tauri build --bundles app

app="$root/target/release/bundle/macos/oxfer.app"
[[ -d "$app" ]] || { echo "build did not produce $app" >&2; exit 1; }

if [[ $install == 0 ]]; then
    echo "built $app"
    exit 0
fi

target="$dest/oxfer.app"
if pgrep -xq oxfer-gui; then
    echo "==> quitting the running oxfer"
    osascript -e 'quit app id "dev.oxfer.app"' || true
    for _ in {1..20}; do pgrep -xq oxfer-gui || break; sleep 0.25; done
fi
echo "==> installing to $target"
mkdir -p "$dest"
rm -rf "$target"
ditto "$app" "$target"
echo "done: open it from Launchpad or Spotlight, or run \`open -a oxfer\`"
