#!/bin/sh
# Run by Xcode for the CallDanya target: builds the app with cargo for the device or the simulator and copies the
# binary into the bundle, where Xcode then signs it. Xcode's Debug is a cargo debug build, which talks to the dev
# server at localhost:8090 (reachable from the simulator, not a phone); Release talks to the real server.
set -eu

case "$PLATFORM_NAME" in
    iphoneos) triple=aarch64-apple-ios ;;
    iphonesimulator) triple=aarch64-apple-ios-sim ;;
    *) echo "error: unsupported platform $PLATFORM_NAME" >&2; exit 1 ;;
esac
if [ "$CONFIGURATION" = Release ]; then profile=release; flag=--release; else profile=debug; flag=; fi

native="$PROJECT_DIR/.."
# Xcode's environment (SDKROOT, deployment targets, compiler flags) is for its own iOS build and would leak into the
# host build scripts cargo compiles, so cargo gets a clean one
env -i PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin" HOME="$HOME" \
    ${LK_CUSTOM_WEBRTC:+LK_CUSTOM_WEBRTC="$LK_CUSTOM_WEBRTC"} \
    cargo build --manifest-path "$native/Cargo.toml" -p call-danya --bin call-danya --target "$triple" $flag

mkdir -p "$TARGET_BUILD_DIR/$EXECUTABLE_FOLDER_PATH"
cp "$native/target/$triple/$profile/call-danya" "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"

# sherpa-onnx (speech) only comes as a framework on iOS; the binary finds it through @executable_path/Frameworks
if [ "$PLATFORM_NAME" = iphoneos ]; then slice=ios-arm64; else slice=ios-arm64_x86_64-simulator; fi
framework=$(find "$native/target/sherpa-onnx-prebuilt" -path "*/SherpaOnnxC.xcframework/$slice/SherpaOnnxC.framework" | head -n 1)
[ -n "$framework" ] || { echo "error: SherpaOnnxC.framework ($slice) not found" >&2; exit 1; }
frameworks="$TARGET_BUILD_DIR/$FRAMEWORKS_FOLDER_PATH"
mkdir -p "$frameworks"
rm -rf "$frameworks/SherpaOnnxC.framework"
cp -R "$framework" "$frameworks/"
# Xcode signs only the app itself; what a script adds is signed here (AltStore signs everything again anyway)
if [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
    codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" "$frameworks/SherpaOnnxC.framework"
fi

# Chinese for Slint: iOS draws it only with PingFang, which Slint's font library rejects (app/src/ios.rs). Noto Sans SC
# from Google Fonts, pinned to a commit and checked, downloaded once into target/
font_commit=a85815a42757630ce188fdad368c2dfc444d4773
font_sha256=a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da
font="$native/target/fonts/NotoSansSC-$font_commit.ttf"
if [ ! -f "$font" ]; then
    mkdir -p "$(dirname "$font")"
    curl -fsSL -o "$font.part" \
        "https://raw.githubusercontent.com/google/fonts/$font_commit/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf"
    echo "$font_sha256  $font.part" | shasum -a 256 -c - >/dev/null || {
        echo "error: NotoSansSC.ttf doesn't match its SHA-256" >&2
        rm -f "$font.part"
        exit 1
    }
    mv "$font.part" "$font"
fi
cp "$font" "$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/NotoSansSC.ttf"
