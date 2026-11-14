#!/usr/bin/env bash
# Print the Changes.md section of one version, for the GitHub release
# (Plan 5.2): the lines under "## <version>" up to the next "## " heading.
#   scripts/release_notes.sh 1.0.0
set -euo pipefail
version=${1:?usage: release_notes.sh VERSION}
root=$(cd "$(dirname "$0")/.." && pwd)
notes=$(awk -v h="## $version" '
    index($0, h) == 1 && (length($0) == length(h) || substr($0, length(h) + 1, 1) == " ") { f = 1; next }
    /^## / { f = 0 }
    f' "$root/Changes.md")
if [ -z "${notes//[[:space:]]/}" ]; then
    echo "no section \"## $version\" in Changes.md" >&2
    exit 1
fi
printf '%s\n' "$notes"
