#!/bin/sh
# Builds for Windows from this Mac with cargo-xwin (clang-cl and lld-link, with Microsoft's CRT and SDK, which it
# downloads once into ~/Library/Caches/cargo-xwin), so Windows-only code is linted and Bergamot's C++ compiled without
# the Windows PC. Needs `brew install llvm lld ninja` and `cargo install --locked cargo-xwin`.
#
#   dev/xwin.sh                     clippy on the whole workspace, as CI's would be on Windows
#   dev/xwin.sh build -p engine     any cargo command, for x86_64-pc-windows-msvc
#
# The app itself doesn't link here: Skia has no prebuilt for the static C runtime the app uses (.cargo/config.toml),
# and building it needs Visual Studio. Clippy links nothing, so it borrows Skia's prebuilt for the dynamic one. Running
# the app, and its speed, are still for the Windows PC.
set -eu
cd "$(dirname "$0")/.."

export PATH="/opt/homebrew/opt/llvm/bin:/opt/homebrew/opt/lld/bin:$PATH"
if [ -z "${SKIA_BINARIES_URL:-}" ]; then
    version=$(grep -A1 '^name = "skia-bindings"' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')
    SKIA_BINARIES_URL=$(curl -fsSL "https://api.github.com/repos/rust-skia/skia-binaries/releases/tags/$version" |
        grep -o '"browser_download_url": *"[^"]*-x86_64-pc-windows-msvc-d3d-ganesh-gl-jpegd-jpege-pdf\.tar\.gz"' |
        sed 's/.*"\(https[^"]*\)"/\1/')
    export SKIA_BINARIES_URL
fi

if [ $# -eq 0 ]; then
    set -- clippy --workspace --all-targets -- -D warnings
fi
command=$1
shift
exec cargo xwin "$command" --release --target x86_64-pc-windows-msvc --target-dir target/xwin "$@"
