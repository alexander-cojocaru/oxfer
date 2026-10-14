#!/usr/bin/env bash
# Sign a built oxfer.app with a stable identity and wrap it in a drag-to-install
# .dmg (the app next to an /Applications shortcut).
#
#   scripts/package-macos-dmg.sh <oxfer.app> <out.dmg>
#
# Signs as $OXFER_SIGN_IDENTITY (default "oxfer local signing", the certificate
# scripts/install-macos-app.sh creates), looked up in $OXFER_SIGN_KEYCHAIN or
# the default keychain search list. Using the same identity for every release
# keeps macOS keychain and folder permissions across updates.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
fi
app="$1"
dmg="$2"
identity="${OXFER_SIGN_IDENTITY:-oxfer local signing}"
keychain=()
[[ -n "${OXFER_SIGN_KEYCHAIN:-}" ]] && keychain=(--keychain "$OXFER_SIGN_KEYCHAIN")

[[ -d "$app" ]] || { echo "no app bundle at $app" >&2; exit 1; }

echo "==> signing $(basename "$app") as \"$identity\""
codesign --force --deep --identifier dev.oxfer.app ${keychain[@]+"${keychain[@]}"} --sign "$identity" "$app"
codesign --verify --deep --strict "$app"

echo "==> creating $dmg"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/oxfer.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname oxfer -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
codesign --force ${keychain[@]+"${keychain[@]}"} --sign "$identity" "$dmg"
echo "built $dmg"
