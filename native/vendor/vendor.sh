#!/bin/sh
# Rebuilds a vendored crate: the crate as published on crates.io, plus our <crate>.patch. Why each one is vendored
# and how to update it: vendor/README.md.
#
#   vendor/vendor.sh webrtc-sys             rebuild it at the version in vendor/webrtc-sys/Cargo.toml
#   vendor/vendor.sh webrtc-sys 0.3.54      rebuild it at another version (then fix what the patch no longer applies to)
#   vendor/vendor.sh --check                fail if any vendored crate isn't the published crate plus its patch (run by CI)
set -eu
cd "$(dirname "$0")"

# Downloads <crate> at <version> into $work and applies its patch; prints where it is
prepare() {
    curl -fsSL --retry 3 "https://static.crates.io/crates/$1/$1-$2.crate" | tar -xz -C "$work"
    crate="$work/$1-$2"
    # Packaging leftovers: cargo writes these when publishing, and the original Cargo.toml isn't the one that's built
    rm -f "$crate/.cargo_vcs_info.json" "$crate/.cargo-ok" "$crate/Cargo.lock" "$crate/Cargo.toml.orig"
    patch -s -p1 -d "$crate" < "$1.patch"
}

version_of() {
    sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$1/Cargo.toml"
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ "${1:-}" = --check ]; then
    for patch in *.patch; do
        name=${patch%.patch}
        version=$(version_of "$name")
        prepare "$name" "$version"
        diff -r "$crate" "$name" > /dev/null ||
            { echo "vendor/$name isn't $name $version plus $patch; run vendor/vendor.sh $name" >&2; exit 1; }
    done
    exit 0
fi

name=${1:?usage: vendor.sh <crate> [version] | --check}
version=${2:-$(version_of "$name")}
prepare "$name" "$version"
rm -rf "$name"
mv "$crate" "$name"
echo "vendor/$name is now $name $version plus $name.patch"
