#!/bin/sh
# Builds the unsigned .ipa and uploads it to the releases bucket, which the server serves at
# https://call.danendz.com/updates (server/updates.ts): call-danya-<version>.ipa, call-danya.ipa (the same file under a
# name that stays, for the website), the icon AltStore shows and then altstore.json, the AltStore
# source that AltStore and SideStore read (add the source once, at that URL) and the app checks (app/src/ios/update.rs).
# Unsigned, since a free Apple ID can't sign for others: AltStore or SideStore signs it on the phone with the user's
# own Apple ID. Run by CI (native.yml) on macOS when app/Cargo.toml's version isn't published for iOS yet; skips if it
# is, so a rerun is safe. `release.sh build` only builds the .ipa into target/ios, for a `build/**` branch.
#
# Needs: Xcode, xcodegen, the aarch64-apple-ios Rust target, the aws CLI; RELEASES_ENDPOINT, RELEASES_BUCKET,
# RELEASES_KEY_ID, RELEASES_SECRET.
set -eu
cd "$(dirname "$0")/../.."
feed=https://call.danendz.com/updates
version=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml)
# The build number, as Android counts it: 1.2.3 → 10203
code=$(echo "$version" | awk -F. '{ print $1 * 10000 + $2 * 100 + $3 }')
ipa="call-danya-$version.ipa"
out=target/ios

if [ "${1:-}" != build ] && curl -fsS "$feed/altstore.json" | grep -q "\"version\": *\"$version\""; then
    echo "$version is already published on ios"
    exit 0
fi

(cd ios && xcodegen)
xcodebuild -project ios/CallDanya.xcodeproj -scheme CallDanya -configuration Release -sdk iphoneos \
    -derivedDataPath ios/build MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$code" CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO \
    CODE_SIGN_IDENTITY= -quiet build

rm -rf "$out"
mkdir -p "$out/Payload"
cp -R ios/build/Build/Products/Release-iphoneos/CallDanya.app "$out/Payload/"
(cd "$out" && zip -qry "$ipa" Payload && rm -rf Payload)
size=$(stat -f %z "$out/$ipa")
sha256=$(shasum -a 256 "$out/$ipa" | cut -d' ' -f1)
cat > "$out/altstore.json" <<JSON
{
  "name": "Call Danya",
  "identifier": "com.danendz.call-danya.source",
  "apps": [
    {
      "name": "Call Danya",
      "bundleIdentifier": "com.danendz.call-danya",
      "developerName": "Call Danya",
      "localizedDescription": "Video calls with live subtitles and translation.",
      "iconURL": "$feed/call-danya-icon.png",
      "versions": [
        {
          "version": "$version",
          "date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
          "size": $size,
          "sha256": "$sha256",
          "downloadURL": "$feed/$ipa",
          "minOSVersion": "17.0"
        }
      ]
    }
  ]
}
JSON
cat "$out/altstore.json"
[ "${1:-}" = build ] && exit 0

export AWS_ACCESS_KEY_ID="$RELEASES_KEY_ID" AWS_SECRET_ACCESS_KEY="$RELEASES_SECRET"
export AWS_DEFAULT_REGION="${RELEASES_REGION:-auto}"
s3() { aws s3 --endpoint-url "$RELEASES_ENDPOINT" "$@"; }
s3 cp "$out/$ipa" "s3://$RELEASES_BUCKET/$ipa" --content-type application/octet-stream
s3 cp "$out/$ipa" "s3://$RELEASES_BUCKET/call-danya.ipa" --content-type application/octet-stream
# AltStore requires an icon
s3 cp ios/Assets.xcassets/AppIcon.appiconset/AppIcon.png "s3://$RELEASES_BUCKET/call-danya-icon.png" --content-type image/png
# Last, so the source never names an .ipa that isn't there yet
s3 cp "$out/altstore.json" "s3://$RELEASES_BUCKET/altstore.json" --content-type application/json

# Keeps the three newest versioned .ipa files, as for Android
# (macOS's head can't drop the last lines)
s3 ls "s3://$RELEASES_BUCKET/" | awk '{ print $4 }' | sed -n 's/^call-danya-\([0-9]*\.[0-9]*\.[0-9]*\)\.ipa$/\1/p' |
    sort -t. -k1,1n -k2,2n -k3,3n | awk '{ kept[NR] = $0 } END { for (i = 1; i <= NR - 3; i++) print kept[i] }' |
    while read -r old; do
        s3 rm "s3://$RELEASES_BUCKET/call-danya-$old.ipa"
    done
