#!/bin/sh
# Rebuilds vendor/webrtc-sys: LiveKit's webrtc-sys as published on crates.io, plus our webrtc-sys.patch. Why it's
# vendored and how to update it: vendor/README.md.
#
#   vendor/webrtc-sys.sh            rebuild it at the version in vendor/webrtc-sys/Cargo.toml
#   vendor/webrtc-sys.sh 0.3.54     rebuild it at another version (then fix what the patch no longer applies to)
#   vendor/webrtc-sys.sh --check    fail if vendor/webrtc-sys isn't the published crate plus the patch (run by CI)
set -eu
cd "$(dirname "$0")"
check=false
case "${1:-}" in
    --check) check=true; version= ;;
    *) version=${1:-} ;;
esac
[ -n "$version" ] || version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' webrtc-sys/Cargo.toml)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 "https://static.crates.io/crates/webrtc-sys/webrtc-sys-$version.crate" | tar -xz -C "$work"
crate="$work/webrtc-sys-$version"
# Packaging leftovers: cargo writes these when publishing, and the original Cargo.toml isn't the one that's built
rm -f "$crate/.cargo_vcs_info.json" "$crate/.cargo-ok" "$crate/Cargo.lock" "$crate/Cargo.toml.orig"
patch -s -p1 -d "$crate" < webrtc-sys.patch

if $check; then
    diff -r "$crate" webrtc-sys > /dev/null ||
        { echo "vendor/webrtc-sys isn't webrtc-sys $version plus webrtc-sys.patch; run vendor/webrtc-sys.sh" >&2; exit 1; }
else
    rm -rf webrtc-sys
    mv "$crate" webrtc-sys
    echo "vendor/webrtc-sys is now webrtc-sys $version plus webrtc-sys.patch"
fi
