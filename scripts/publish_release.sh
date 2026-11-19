#!/usr/bin/env bash
# Publish the draft release of a tag, but only after checking it (SPEC-002,
# Plan 7.3, TC-123): exactly the three archives and their .sha256 files are
# attached, and every downloaded archive matches its checksum. On any
# failure the release stays a draft and the script exits non-zero.
#   scripts/publish_release.sh v1.2.0           (needs GH_TOKEN, gh)
#   scripts/publish_release.sh --expected 1.2.0 (only print the file list)
set -euo pipefail

expected() {
    local v=$1 f
    for f in "virt-interact-$v-linux-x86_64.tar.gz" \
        "virt-interact-$v-windows-x86_64.zip" \
        "virt-interact-$v-macos-universal.zip"; do
        printf '%s\n%s.sha256\n' "$f" "$f"
    done | LC_ALL=C sort
}

if [ "${1:-}" = "--expected" ]; then
    expected "${2:?usage: publish_release.sh --expected VERSION}"
    exit 0
fi
tag=${1:?usage: publish_release.sh TAG}
want=$(expected "${tag#v}")
got=$(gh release view "$tag" --json assets --jq '.assets[].name' | LC_ALL=C sort)
if [ "$got" != "$want" ]; then
    echo "::error::the files on release $tag are not the expected six; it stays a draft"
    diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") || true
    exit 1
fi
dir=$(mktemp -d)
gh release download "$tag" --dir "$dir"
(cd "$dir" && sha256sum -c -- *.sha256)
gh release edit "$tag" --draft=false --latest
echo "published $tag"
