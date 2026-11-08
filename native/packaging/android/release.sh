#!/bin/sh
# Builds the signed release APK and uploads it to the releases bucket, which the server serves at
# https://call.danendz.com/updates (server/updates.ts): call-danya-<version>.apk, call-danya.apk (the same file under
# a name that stays, for first installs) and then android.json, which the app checks (Updater.kt). Run by CI
# (native.yml) on Linux when app/Cargo.toml's version isn't published for Android yet; skips if it is, so a rerun
# is safe. `release.sh build` only builds the signed APK into target/android, for a `build/**` branch.
#
# Needs: the Android SDK with NDK 28.2 and platform 35, cargo-ndk, the aws CLI; RELEASES_ENDPOINT, RELEASES_BUCKET,
# RELEASES_KEY_ID, RELEASES_SECRET; ANDROID_KEYSTORE_BASE64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS,
# ANDROID_KEY_PASSWORD, the release key (an APK signed with any other can't update the installed app).
set -eu
cd "$(dirname "$0")/../.."
feed=https://call.danendz.com/updates
version=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml)
# As android/app/build.gradle.kts counts it: 1.2.3 → 10203
code=$(echo "$version" | awk -F. '{ print $1 * 10000 + $2 * 100 + $3 }')
apk="call-danya-$version.apk"
out=target/android

if [ "${1:-}" != build ] && curl -fsS "$feed/android.json" | grep -q "\"version\": *\"$version\""; then
    echo "$version is already published on android"
    exit 0
fi

# Without the key, Gradle would make an unsigned APK that no phone installs
[ -n "${ANDROID_KEYSTORE_BASE64:-}" ] || { echo "::error::The ANDROID_KEYSTORE_BASE64 secret isn't set"; exit 1; }
keystore="$RUNNER_TEMP/release.jks"
printf %s "$ANDROID_KEYSTORE_BASE64" | base64 --decode > "$keystore"
export ANDROID_KEYSTORE="$keystore"
(cd android && ./gradlew --no-daemon -q assembleRelease)
rm "$keystore"

rm -rf "$out"
mkdir -p "$out"
cp android/app/build/outputs/apk/release/app-release.apk "$out/$apk"
sha256=$(sha256sum "$out/$apk" | cut -d' ' -f1)
printf '{\n  "version": "%s",\n  "code": %s,\n  "file": "%s",\n  "sha256": "%s"\n}\n' \
    "$version" "$code" "$apk" "$sha256" > "$out/android.json"
cat "$out/android.json"
[ "${1:-}" = build ] && exit 0

export AWS_ACCESS_KEY_ID="$RELEASES_KEY_ID" AWS_SECRET_ACCESS_KEY="$RELEASES_SECRET"
export AWS_DEFAULT_REGION="${RELEASES_REGION:-auto}"
s3() { aws s3 --endpoint-url "$RELEASES_ENDPOINT" "$@"; }
type=application/vnd.android.package-archive
s3 cp "$out/$apk" "s3://$RELEASES_BUCKET/$apk" --content-type "$type"
s3 cp "$out/$apk" "s3://$RELEASES_BUCKET/call-danya.apk" --content-type "$type"
# Last, so the feed never names an APK that isn't there yet
s3 cp "$out/android.json" "s3://$RELEASES_BUCKET/android.json" --content-type application/json

# Keeps the three newest versioned APKs, as vpk does for desktop
s3 ls "s3://$RELEASES_BUCKET/" | awk '{ print $4 }' | grep -E '^call-danya-[0-9]+\.[0-9]+\.[0-9]+\.apk$' |
    sort -V | head -n -3 | while read -r old; do
        s3 rm "s3://$RELEASES_BUCKET/$old"
    done
