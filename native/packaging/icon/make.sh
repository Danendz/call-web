#!/bin/sh
# Renders the app icon for every platform from background.svg and foreground.svg, into the files each one reads.
# The output is committed, so builds need none of this; run it again after changing the artwork.
#
# Needs: resvg (`brew install resvg`), and macOS for iconutil.
set -eu
cd "$(dirname "$0")"
here=$(pwd)
native=../..
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# One layout of the artwork as an SVG around the two layers: $1 name, $2 what goes inside the 1024 square
variant() {
    cat > "$tmp/$1.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 1024 1024" width="1024" height="1024">
$2
</svg>
SVG
}
layers='<image width="1024" height="1024" xlink:href="background.svg"/><image width="1024" height="1024" xlink:href="foreground.svg"/>'
# Full bleed: iOS rounds the corners itself
variant full "$layers"
# A rounded tile filling the square: Windows, the window icon, the web
variant tile "<clipPath id=\"c\"><rect width=\"1024\" height=\"1024\" rx=\"224\"/></clipPath><g clip-path=\"url(#c)\">$layers</g>"
# macOS's grid: an 824 tile with a shadow, inset 100
variant mac "<defs><filter id=\"s\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"125%\"><feDropShadow dx=\"0\" dy=\"10\" stdDeviation=\"12\" flood-opacity=\"0.35\"/></filter><clipPath id=\"c\"><rect width=\"1024\" height=\"1024\" rx=\"230\"/></clipPath></defs><g filter=\"url(#s)\"><g transform=\"translate(100 100) scale(0.8046875)\"><g clip-path=\"url(#c)\">$layers</g></g></g>"
# Android's adaptive icon draws its own mask, and only the middle 66 of its 108 dp is sure to show
variant android-fg '<g transform="translate(512 512) scale(0.68) translate(-512 -512)"><image width="1024" height="1024" xlink:href="foreground.svg"/></g>'

render() { resvg --resources-dir "$here" -w "$3" -h "$3" "$tmp/$1.svg" "$2"; }

ios="$native/ios/Assets.xcassets/AppIcon.appiconset"
render full "$ios/AppIcon.png" 1024

mkdir -p "$tmp/AppIcon.iconset"
for size in 16 32 128 256 512; do
    render mac "$tmp/AppIcon.iconset/icon_${size}x${size}.png" "$size"
    render mac "$tmp/AppIcon.iconset/icon_${size}x${size}@2x.png" $((size * 2))
done
iconutil -c icns -o "$native/packaging/macos/AppIcon.icns" "$tmp/AppIcon.iconset"

for size in 16 24 32 48 64 128 256; do
    render tile "$tmp/ico-$size.png" "$size"
done
# An .ico is a directory of PNGs; sips only writes one size
python3 - "$native/packaging/windows/call-danya.ico" "$tmp"/ico-16.png "$tmp"/ico-24.png "$tmp"/ico-32.png \
    "$tmp"/ico-48.png "$tmp"/ico-64.png "$tmp"/ico-128.png "$tmp"/ico-256.png <<'PY'
import struct, sys
pngs = [open(p, "rb").read() for p in sys.argv[2:]]
offset = 6 + 16 * len(pngs)
out = struct.pack("<HHH", 0, 1, len(pngs))
for png in pngs:
    w, h = struct.unpack(">II", png[16:24])
    out += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(png), offset)
    offset += len(png)
open(sys.argv[1], "wb").write(out + b"".join(pngs))
PY

render tile "$native/app/ui/app-icon.png" 256

res="$native/android/app/src/main/res/mipmap-xxxhdpi"
resvg --resources-dir "$here" -w 432 -h 432 background.svg "$res/ic_launcher_background.png"
render android-fg "$res/ic_launcher_foreground.png" 432

web="$native/../web/public"
render tile "$web/favicon.png" 64
render full "$web/apple-touch-icon.png" 180
