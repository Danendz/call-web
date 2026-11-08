#!/bin/sh
# Packs the release build with Velopack and uploads it to the releases bucket, which the server serves at
# https://call.danendz.com/updates (server/updates.ts). Run by CI (native.yml) on macOS and Windows after
# `cargo build --release`, when app/Cargo.toml's version isn't published yet. Skips a channel that already has it,
# so a rerun after a half-finished release only uploads what's missing.
#
# Needs: vpk on PATH; RELEASES_ENDPOINT, RELEASES_BUCKET, RELEASES_KEY_ID, RELEASES_SECRET; on macOS also
# MACOS_CERT_P12 (base64) and MACOS_CERT_PASSWORD, our self-signed "Call Danya" code-signing certificate.
set -eu
cd "$(dirname "$0")/.."
feed=https://call.danendz.com/updates
version=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml)
out=target/velopack
case "$(uname -s)" in
    Darwin) channel=osx ;;
    *) channel=win ;;
esac

if curl -fsS "$feed/releases.$channel.json" | grep -q "\"Version\": *\"$version\""; then
    echo "$version is already published on $channel"
    exit 0
fi

rm -rf "$out"
# The previous release, so vpk can make a delta against it; nothing yet before the first release
vpk download http --url "$feed" --channel "$channel" --allowEmptyChannel -o "$out"

pack() {
    vpk pack --packId CallDanya --packTitle "Call Danya" --packAuthors Danendz --packVersion "$version" -o "$out" "$@"
}
if [ "$channel" = osx ]; then
    packaging/macos/bundle.sh
    # A stable certificate keeps the camera, mic and screen recording permissions across updates; ad-hoc signing
    # changes the app's identity with every build, so macOS would ask again after each one
    keychain="$RUNNER_TEMP/sign.keychain-db"
    printf %s "$MACOS_CERT_P12" | base64 --decode > "$RUNNER_TEMP/sign.p12"
    security create-keychain -p ci "$keychain"
    security set-keychain-settings "$keychain"
    security unlock-keychain -p ci "$keychain"
    security import "$RUNNER_TEMP/sign.p12" -k "$keychain" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k ci "$keychain" > /dev/null
    rm "$RUNNER_TEMP/sign.p12"
    pack --packDir "target/release/Call Danya.app" --mainExe call-danya \
        --signAppIdentity "Call Danya" --keychain "$keychain" --signEntitlements packaging/macos/call-danya.entitlements
else
    rm -rf target/velopack-app
    mkdir -p target/velopack-app
    cp target/release/call-danya.exe target/velopack-app/
    pack --packDir target/velopack-app --mainExe call-danya.exe
fi

vpk upload s3 -o "$out" --channel "$channel" --endpoint "$RELEASES_ENDPOINT" --bucket "$RELEASES_BUCKET" \
    --keyId "$RELEASES_KEY_ID" --secret "$RELEASES_SECRET" --keepMaxReleases 3
