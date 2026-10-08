#!/usr/bin/env bash
# Render the Homebrew formula from the release archives in a directory.
#   scripts/gen-formula.sh <owner/repo> <version> <dir-with-archives> > oxfer.rb
set -euo pipefail
repo=${1:?owner/repo}; version=${2:?version (no leading v)}; dir=${3:?archive dir}

sha() {
  local f="$dir/oxfer-v$version-$1.tar.gz"
  [ -f "$f" ] || { echo "missing archive: $f" >&2; exit 1; }
  if command -v sha256sum >/dev/null; then sha256sum "$f" | cut -d' ' -f1; else shasum -a 256 "$f" | cut -d' ' -f1; fi
}

here=$(cd "$(dirname "$0")" && pwd)
sed -e "s|@REPO@|$repo|g" -e "s|@VERSION@|$version|g" \
    -e "s|@SHA_MACOS_ARM@|$(sha aarch64-apple-darwin)|" \
    -e "s|@SHA_MACOS_INTEL@|$(sha x86_64-apple-darwin)|" \
    -e "s|@SHA_LINUX@|$(sha x86_64-unknown-linux-gnu)|" \
    "$here/../packaging/homebrew/oxfer.rb.in"
