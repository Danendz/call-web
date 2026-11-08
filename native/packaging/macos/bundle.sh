#!/bin/sh
# Wraps the release binary in Call Danya.app and zips it. A bundle with usage descriptions is what makes macOS ask
# for the mic and camera on the app's behalf; a bare binary would borrow Terminal's permissions instead. Ad-hoc
# signed only (real signing is M6), so the first launch needs right-click → Open.
set -eu
cd "$(dirname "$0")/../.."
app=target/release/"Call Danya.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp packaging/macos/Info.plist "$app/Contents/"
cp target/release/call-danya "$app/Contents/MacOS/"
codesign --force --sign - "$app"
ditto -c -k --keepParent "$app" target/release/call-danya-macos.zip
