#!/usr/bin/env bash
# Render the updater manifest (latest.json) read by the app's update check.
#   scripts/gen-latest-json.sh <owner/repo> <tag> <app.tar.gz> <app.tar.gz.sig> <notes.md> > latest.json
# The archive is a universal build, so both macOS architectures point to it.
set -euo pipefail
repo=${1:?owner/repo}; tag=${2:?tag (with leading v)}; archive=${3:?app.tar.gz}
sig=${4:?signature file}; notes=${5:?release notes file}

[ -f "$archive" ] && [ -f "$sig" ] && [ -f "$notes" ] || { echo "missing input file" >&2; exit 1; }
url="https://github.com/$repo/releases/download/$tag/$(basename "$archive")"
platform=$(jq -n --arg url "$url" --rawfile sig "$sig" '{url: $url, signature: ($sig | rtrimstr("\n"))}')

jq -n --arg version "${tag#v}" --rawfile notes "$notes" --argjson platform "$platform" \
  '{version: $version, notes: $notes, pub_date: (now | todate),
    platforms: {"darwin-aarch64": $platform, "darwin-x86_64": $platform}}'
