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
  encoder factory's `Hardware` backend, so `codec.rs` sees hardware H.264 and Auto picks it. Upstream reaches the GPU's
  encoder only through NVENC, and only on Linux.

- `webrtc-sys/` is the crate exactly as published on crates.io, plus `webrtc-sys.patch`. Don't edit it by hand:
  change the patch and run `./webrtc-sys.sh`. CI runs `./webrtc-sys.sh --check` and fails when the two differ.
- `webrtc-sys.patch` is everything we changed: `capture_processor.{h,cpp,rs}`, two lines in `build.rs` and `lib.rs`,
  and the factory creating its audio processing with the hook in `peer_connection_factory.cpp`; then
  `media_foundation_encoder.{h,cpp}`, the Windows lines that build and link it in `build.rs`, and its registration in
  `video_encoder_factory.cpp`.
- `webrtc-sys-build` (which downloads the WebRTC prebuilt) is unchanged and comes from crates.io as usual.

**Why vendored rather than a git dependency on a fork:** cargo fetches a git dependency's whole repository with all
its submodules. LiveKit's `rust-sdks` is ~100 MB and one submodule lives on `chromium.googlesource.com`, which can't
be reached from mainland China, so builds hung there; CI fetched it on every cold build, and a GitHub outage failed a
release. A path dependency needs no fetch, `Cargo.lock` stays plain, and a change to the hook is reviewed in the same
PR as the Rust that uses it.

### Updating to a new webrtc-sys

Only when `libwebrtc` (in `crates/engine/Cargo.toml`) moves to a release that needs a newer `webrtc-sys`; its
`Cargo.toml` names the version.

1. `./webrtc-sys.sh <version>` from `native/vendor`. It downloads that version from crates.io and applies the patch.
2. If the patch doesn't apply, fix it by hand in the new `webrtc-sys/` (the `.rej` files say what didn't fit; the
   hook usually only has to follow changes to how `peer_connection_factory.cpp` builds its audio processing), then
   write it back from a clean copy of the new version:

   ```
   mkdir /tmp/ws && curl -fsSL https://static.crates.io/crates/webrtc-sys/webrtc-sys-<version>.crate | tar -xz -C /tmp/ws
   diff -ruN -x Cargo.toml.orig -x .cargo_vcs_info.json -x .cargo-ok -x Cargo.lock -x '*.orig' -x '*.rej' \
       /tmp/ws/webrtc-sys-<version> webrtc-sys | sed 's|/tmp/ws/webrtc-sys-[^/]*/|a/|; s| webrtc-sys/| b/|' > webrtc-sys.patch
   ./webrtc-sys.sh && ./webrtc-sys.sh --check
   ```

3. If `webrtc-sys-build` moved too, update its `=` pin in `app/Cargo.toml` to the version `webrtc-sys` asks for.
4. `cargo build` so `Cargo.lock` follows, then test a call with noise suppression on and off: the hook only shows in
   what the other side hears.

If LiveKit ever adds a capture post-processing hook of its own, move `crates/engine` to it and delete this folder and
the `[patch.crates-io]` entry.
