#!/bin/sh
# Wraps the release binary in Call Danya.app and zips it. A bundle with usage descriptions is what makes macOS ask
# for the mic and camera on the app's behalf; a bare binary would borrow Terminal's permissions instead. Signed
# ad-hoc here: a release (packaging/release.sh) re-signs it with our own certificate.
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml)
app=target/release/"Call Danya.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp packaging/macos/Info.plist "$app/Contents/"
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$app/Contents/Info.plist"
cp target/release/call-danya "$app/Contents/MacOS/"
codesign --force --sign - "$app"
ditto -c -k --keepParent "$app" target/release/call-danya-macos.zip
