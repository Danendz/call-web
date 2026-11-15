# Vendored crates

Third-party crates we had to change, kept here as source and patched in from `native/Cargo.toml`
(`[patch.crates-io]`). They're excluded from the workspace, so `cargo fmt` and `clippy` leave them alone: they keep
their upstream style, which keeps our patch small.

## webrtc-sys

LiveKit's `webrtc-sys` (the C++ shim between `libwebrtc` and Google's WebRTC) with two additions:

- A hook that hands each 10 ms of mic audio to Rust after WebRTC's echo canceller and gain control, where DeepFilterNet
  runs (`crates/denoise`, installed by `crates/engine/src/media.rs`), and a switch for WebRTC's own noise suppression,
  which is off while ours is on. Upstream has no way to do either from Rust.
- On Windows, a hardware H.264 encoder through Media Foundation (`media_foundation_encoder.{h,cpp}`), registered as the
  encoder factory's `Hardware` backend, so `codec.rs` sees hardware H.264 and Auto picks it, and a decoder on the GPU
  (`media_foundation_decoder.{h,cpp}`) in front of FFmpeg's, which takes over wherever it fails. Upstream reaches the
  GPU's encoder only through NVENC, and only on Linux, and decodes H.264 on Windows only in software.
- A switch for FlexFEC on video (`flexfec.{h,cpp,rs}`): the factory always carries its field trials view, which reads
  the switch on every lookup, so a peer connection made after it changes gets it. Upstream sets field trials only
  when a factory is made.

- `webrtc-sys/` is the crate exactly as published on crates.io, plus `webrtc-sys.patch`. Don't edit it by hand:
  change the patch and run `./vendor.sh webrtc-sys`. CI runs `./vendor.sh --check` and fails when any vendored crate
  and its patch differ.
- `webrtc-sys.patch` is everything we changed: `capture_processor.{h,cpp,rs}`, two lines in `build.rs` and `lib.rs`,
  and the factory creating its audio processing with the hook in `peer_connection_factory.cpp`; then
  `media_foundation_encoder.{h,cpp}` and `media_foundation_decoder.{h,cpp}`, the Windows lines that build and link
  them in `build.rs`, and their registration in `video_encoder_factory.cpp` and `video_decoder_factory.cpp`; then
  `flexfec.{h,cpp,rs}`, its lines in `build.rs` and `lib.rs`, and the factory adding its view in
  `peer_connection_factory.cpp`.
- `webrtc-sys-build` (which downloads the WebRTC prebuilt) is unchanged and comes from crates.io as usual.

**Why vendored rather than a git dependency on a fork:** cargo fetches a git dependency's whole repository with all
its submodules. LiveKit's `rust-sdks` is ~100 MB and one submodule lives on `chromium.googlesource.com`, which can't
be reached from mainland China, so builds hung there; CI fetched it on every cold build, and a GitHub outage failed a
release. A path dependency needs no fetch, `Cargo.lock` stays plain, and a change to the hook is reviewed in the same
PR as the Rust that uses it.

### Updating to a new webrtc-sys

Only when `libwebrtc` (in `crates/engine/Cargo.toml`) moves to a release that needs a newer `webrtc-sys`; its
`Cargo.toml` names the version.

1. `./vendor.sh webrtc-sys <version>` from `native/vendor`. It downloads that version from crates.io and applies the patch.
2. If the patch doesn't apply, fix it by hand in the new `webrtc-sys/` (the `.rej` files say what didn't fit; the
   hook usually only has to follow changes to how `peer_connection_factory.cpp` builds its audio processing), then
   write it back from a clean copy of the new version:

   ```
   mkdir /tmp/ws && curl -fsSL https://static.crates.io/crates/webrtc-sys/webrtc-sys-<version>.crate | tar -xz -C /tmp/ws
   diff -ruN -x Cargo.toml.orig -x .cargo_vcs_info.json -x .cargo-ok -x Cargo.lock -x '*.orig' -x '*.rej' \
       /tmp/ws/webrtc-sys-<version> webrtc-sys | sed 's|/tmp/ws/webrtc-sys-[^/]*/|a/|; s| webrtc-sys/| b/|' > webrtc-sys.patch
   ./vendor.sh webrtc-sys && ./vendor.sh --check
   ```

3. If `webrtc-sys-build` moved too, update its `=` pin in `app/Cargo.toml` to the version `webrtc-sys` asks for.
4. `cargo build` so `Cargo.lock` follows, then test a call with noise suppression on and off: the hook only shows in
   what the other side hears.

If LiveKit ever adds a capture post-processing hook of its own, move `crates/engine` to it and delete this folder and
the `[patch.crates-io]` entry.

## i-slint-backend-android-activity

Slint's Android backend, with the renderer picked at runtime instead of at build time. Upstream makes the renderer
once, from the crate's features, and with Vulkan built in it uses Vulkan or fails: the app couldn't fall back to
OpenGL on a phone whose driver can't, or let Settings switch between them. Our patch:

- Always makes the Skia renderer that picks its surface from `requested_graphics_api`, and makes the window's surface
  through one `init_surface`, which falls back to OpenGL ES 3.0 in place (and keeps that) when the API asked for
  can't make one. The app sees the fallback from the rendering notifier (`app/src/window.rs`).
- `set_requested_graphics_api` returns an error and, once there's a window, makes its surface again with the new API,
  so Settings' "Graphics" applies at once.
- Releases the surface on `TerminateWindow`, so each one the app gets back after the background is made fresh, which
  is what `window.rs` checks Vulkan against.

It's pinned to the same `=` version as `slint` in `app/Cargo.toml`, since it reaches into Slint's unstable internals
(`i-slint-core`, `i-slint-renderer-skia`). To move Slint to a new version, run `./vendor.sh
i-slint-backend-android-activity <version>`, fix what no longer applies as for webrtc-sys above, and test on a phone:
launch, background and resume, the screen off and on during a call, and both Graphics choices. Until upstream offers
the runtime choice; then delete this folder and its `[patch.crates-io]` entry.
