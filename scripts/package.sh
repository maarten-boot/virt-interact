#!/usr/bin/env bash
# Build a release archive for one platform (Plan 5.1, SPEC-002, TC-120).
#
#   scripts/package.sh linux|windows|macos [OUT_DIR]   (default OUT_DIR: dist)
#
# Builds the release binary (cargo build --release --locked), stages it with
# the docs and examples, writes the archive and a .sha256 file to OUT_DIR,
# and checks that the archive holds every expected file.
#   linux:   virt-interact-<ver>-linux-x86_64.tar.gz
#   windows: virt-interact-<ver>-windows-x86_64.zip
#   macos:   virt-interact-<ver>-macos-universal.zip  (virt-interact.app,
#            arm64 + x86_64, ad-hoc signed, not notarized; macOS 12+)
# Respects CARGO_TARGET_DIR. Runs on the platform it packages for (macOS
# needs lipo, codesign and ditto); Windows runs it in Git Bash.
set -euo pipefail

os=${1:?usage: package.sh linux|windows|macos [OUT_DIR]}
out=${2:-dist}
root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
name=virt-interact-$version

mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

build() { (cd "$root" && cargo build --release --locked -p vi-gui "$@"); }

# Docs and examples next to the program.
docs() {
    local d=$1
    mkdir -p "$d/examples"
    cp "$root/README.md" "$root/Changes.md" "$d/"
    cp "$root/examples/config.example.toml" "$root/examples/ssh_config.example" "$d/examples/"
    cp "$root/LICENSE" "$d/"
}

sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi
}

case $os in
linux)
    build
    stage=$work/$name
    mkdir -p "$stage"
    cp "$target/release/virt-interact" "$stage/"
    docs "$stage"
    archive=$out/$name-linux-x86_64.tar.gz
    # No local user/group names in the archive (GNU tar); stable order.
    tar --owner=0 --group=0 --numeric-owner --sort=name -czf "$archive" -C "$work" "$name"
    listing=$(tar -tzf "$archive")
    expect=("$name/virt-interact" "$name/README.md" "$name/LICENSE" "$name/examples/config.example.toml")
    ;;
windows)
    build
    stage=$work/$name
    mkdir -p "$stage"
    cp "$target/release/virt-interact.exe" "$stage/"
    docs "$stage"
    archive=$out/$name-windows-x86_64.zip
    rm -f "$archive"
    if command -v 7z >/dev/null; then
        (cd "$work" && 7z a -tzip -bso0 "$archive" "$name")
        listing=$(7z l -ba "$archive")
    else
        (cd "$work" && zip -qr "$archive" "$name")
        listing=$(unzip -l "$archive")
    fi
    expect=("$name/virt-interact.exe" "$name/README.md" "$name/LICENSE" "$name/examples/config.example.toml")
    ;;
macos)
    export MACOSX_DEPLOYMENT_TARGET=12.0
    build --target aarch64-apple-darwin
    build --target x86_64-apple-darwin
    stage=$work/$name
    app=$stage/virt-interact.app
    mkdir -p "$app/Contents/MacOS"
    lipo -create -output "$app/Contents/MacOS/virt-interact" \
        "$target/aarch64-apple-darwin/release/virt-interact" \
        "$target/x86_64-apple-darwin/release/virt-interact"
    sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
    # arm64 code must be signed to run at all; ad-hoc, not notarized.
    codesign --force --deep --sign - "$app"
    codesign --verify --deep --strict "$app"
    lipo -info "$app/Contents/MacOS/virt-interact"
    docs "$stage"
    archive=$out/$name-macos-universal.zip
    rm -f "$archive"
    ditto -c -k --keepParent "$stage" "$archive"
    listing=$(unzip -l "$archive")
    expect=("$name/virt-interact.app/Contents/MacOS/virt-interact"
            "$name/virt-interact.app/Contents/Info.plist" "$name/README.md" "$name/LICENSE"
            "$name/examples/config.example.toml")
    ;;
*)
    echo "unknown platform: $os (linux|windows|macos)" >&2
    exit 2
    ;;
esac

# Normalise Windows separators in 7z listings before checking.
listing=${listing//\\//}
for e in "${expect[@]}"; do
    if ! grep -qF -- "$e" <<<"$listing"; then
        echo "missing in $archive: $e" >&2
        exit 1
    fi
done
(cd "$out" && sha256 "$(basename "$archive")" > "$(basename "$archive").sha256")
echo "packaged: $archive"
cat "$archive.sha256"
