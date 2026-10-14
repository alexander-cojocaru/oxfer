#!/usr/bin/env bash
# Print the CHANGELOG.md section for a version, for use as GitHub release notes.
#
#   scripts/release-notes.sh v0.4.1     # or 0.4.1
set -euo pipefail

[[ $# -eq 1 ]] || { sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
version="${1#v}"
root="$(cd "$(dirname "$0")/.." && pwd)"

notes="$(awk -v v="$version" '
    /^## / { if (found) exit; found = ($2 == v) ; next }
    found
' "$root/CHANGELOG.md" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | sed '/./,$!d')"

if [[ -z "$notes" ]]; then
    echo "no CHANGELOG.md entry for $version" >&2
    exit 1
fi
printf '%s\n' "$notes"
