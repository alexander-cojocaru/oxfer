#!/usr/bin/env bash
# Render the updater manifest (latest.json) read by the app's update check.
#   scripts/gen-latest-json.sh <owner/repo> <tag> <notes.md> <platform>=<archive>... > latest.json
# Each <archive> has a signature next to it (<archive>.sig). A platform is a
# Tauri updater key, several can share one archive (the macOS build is
# universal), e.g.
#   darwin-aarch64,darwin-x86_64=oxfer-v1-macos-universal.app.tar.gz
#   windows-x86_64=oxfer-v1-windows-x64-setup.exe
set -euo pipefail
repo=${1:?owner/repo}; tag=${2:?tag (with leading v)}; notes=${3:?release notes file}
shift 3
[ -f "$notes" ] && [ $# -gt 0 ] || { echo "missing release notes or platform" >&2; exit 1; }

platforms='{}'
for spec in "$@"; do
  keys=${spec%%=*}; archive=${spec#*=}; sig="$archive.sig"
  [ -f "$archive" ] && [ -f "$sig" ] || { echo "missing $archive or its signature" >&2; exit 1; }
  url="https://github.com/$repo/releases/download/$tag/$(basename "$archive")"
  platform=$(jq -n --arg url "$url" --rawfile sig "$sig" '{url: $url, signature: ($sig | rtrimstr("\n"))}')
  platforms=$(jq -n --argjson all "$platforms" --arg keys "$keys" --argjson p "$platform" \
    '$all + ($keys | split(",") | map({key: ., value: $p}) | from_entries)')
done

jq -n --arg version "${tag#v}" --rawfile notes "$notes" --argjson platforms "$platforms" \
  '{version: $version, notes: $notes, pub_date: (now | todate), platforms: $platforms}'
