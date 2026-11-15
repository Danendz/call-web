# Native app performance notes

Where the native app spends CPU and memory, and what was done about it. The shipped items are tracked under **Efficiency** in [ROADMAP.md](ROADMAP.md); this file keeps the measurements and the reasoning behind them.

## Baseline

Release build on an Apple Silicon Mac, local server, fake camera at 15 fps, headless `join` peer on the other side:

| Screen | CPU | Memory |
|---|---|---|
| Start screen, idle | 0.6–0.7% | 150 MB |
| Setup screen (camera preview + mic meter) | ~4.7% | 430 MB |
| In a 2-way call, SenseVoice loaded | 16–21% | 780 MB (1.0 GB peak) |

For scale, the headless `join` peer used 14–21% for the same call with no UI. Most of the in-call cost is WebRTC itself (encoding, decoding, audio processing), and the UI path is the part the app controls. In the in-call profile, the top user-space frames were the GPU texture upload of each RGBA video frame (`agxsTwiddleAddressCommon`), `memset` on fresh frame buffers, and `gettimeofday` from timer polling.

**How to measure:** a release build, `CALL_FAKE_CAMERA=1`, and a local server (`PORT=8090 npm run dev:server`). Run `cargo run -p engine --example join -- http://localhost:8090 <room>` as the other side. Read CPU from Activity Monitor or `top -pid`, memory from `vmmap -summary`, and take hot frames from a `sample` of the process during the call.

## Findings, by expected gain

1. **Video went CPU → RGBA → fresh buffer → GPU every frame.** Each 720p frame allocated a new 3.7 MB `SharedPixelBuffer`, libyuv converted into it, and Skia uploaded the whole RGBA texture. At 30 fps that's ~110 MB/s of allocation, memset and conversion per feed, and the upload was the hottest user-space frame.
   - *Shipped:* a few reused buffers per feed (`make_mut_bytes`; Slint copies only if the UI still holds one), and the self-view mirrored at draw time.
   - *Next:* the wgpu renderer with video handed in as YUV planes and converted by a shader. This removes the CPU conversion and uploads 2.7× less. A YUV shader on the current OpenGL renderer was ruled out because Skia doesn't restore GL state after an outside draw and Android drops the context in the background. The upload is still the hottest frame after buffer reuse.
   - *Shipped (step 1):* Skia draws over wgpu (Metal on macOS) on desktop, with OpenGL as a fallback, CPU video unchanged. Same method as the baseline, OpenGL build vs wgpu build, a few runs each:

     | Screen | OpenGL | wgpu (Metal) |
     |---|---|---|
     | Start screen, idle | 0.5–0.6%, 138–146 MB | 0.5–0.6%, 103–107 MB |
     | Setup screen | 1.7–6.3%, 144–333 MB | 2.5–7.4%, 84–302 MB |
     | In a call | 13.6–24.1%, 775–811 MB | 14.5–15.5%, 386–513 MB |

     CPU is unchanged. The memory drop in a call may partly be Metal's GPU memory being counted differently in `vmmap`'s physical footprint. Setup varies on both, depending on when the camera preview starts inside the sample.

   - *Shipped (step 2):* with wgpu, each frame's Y, U and V planes go up as they are and a shader turns them into the texture Slint draws (`app/src/gpu.rs`); OpenGL keeps converting on the CPU. Measured on an M-series Mac with `sample` in a call receiving a 1280×800 test screen at up to 60 fps plus camera both ways, on VP8: conversion and Skia's RGBA upload were 7% of the app's busy CPU on the CPU path and disappear on the GPU one. Small, because libyuv converts a 1280×800 frame in ~0.2 ms with NEON. VP8's software encoder and decoder were 64–73%: hardware H.264 (the video codec setting) is the bigger win. `CALL_CPU_VIDEO=1` keeps the CPU path for comparing.

   - *Shipped (step 3):* Android draws with Skia over Vulkan, and video goes through the same shader. Measured on a Snapdragon 870 tablet (Android 14) in a release build, receiving the `join` example's test camera and a 1280×800 test screen over H.264 (hardware both ways), whole-app CPU from `/proc/<pid>/stat` over 60 s, 100% = one core:

     | Run | OpenGL, CPU conversion | Vulkan, GPU conversion |
     |---|---|---|
     | Camera only | 90.2% | 89.1% |
     | Camera + screen, 1 | 108.7% at ~6 fps | 108.0% at ~20 fps |
     | Camera + screen, 2 | 118.4% at ~31 fps | 88.0% at 5–14 fps |

     No regression, and no gain that stands out from the noise: the sender's bandwidth estimate set the screen's frame rate differently in every run, so the screen runs can't be compared with each other. As on the Mac, libyuv's NEON conversion is cheap, and Vulkan moves some work onto the UI thread instead (Skia's Vulkan backend, and every GPU submit, which Vulkan needs on one thread). A fair comparison needs a sender held at a fixed resolution and frame rate.

2. **Frames were converted when nobody would see them.** *Shipped:* a feed isn't converted while the window is minimized or covered, or while picture-in-picture shows a different feed.

3. **Speech to chat opened the mic a second time** and ran a second `AudioProcessingModule` with its own echo canceller, fed the other person's decoded audio as the echo reference. *Shipped:* it now reads the call's mic after WebRTC's processing, through the vendored capture hook.

4. **The 30 Hz UI timer ran for the app's whole life**, including the start screen, where it made up almost all of the 0.6%. *Shipped:* 30 Hz on the setup screen for the level meter, 4 Hz in a call, and off elsewhere. Android's `android::sync` runs with it.

5. **Every engine event cloned and published the whole `State`**, and the bridge wrote ~50 properties each time, at least once a second because of stats. *Shipped:* a state that didn't change isn't published, and stats have their own callback.

6. **Stats were formatted and sent to the UI every second**, even with the overlay hidden. The engine only needs the path type for its relay check. *Shipped:* sampled every second, handed to the UI only while the overlay is shown, otherwise every 5 s.

7. **Camera preview on the setup screen** (4.7%, 430 MB): the camera, the level meter and the conversion all run before joining. This matches the web on purpose. A 15 fps preview, and 15 fps for the small self-view in a call, would cut it. It's a UX trade-off, not a bug.

8. **Memory:** in a call, `vmmap` showed 228 MB of freed but still resident heap ("Malloc Small (empty)") after the speech models (SenseVoice ~240 MB, live ~275 MB) were released. *Shipped:* the allocator is asked to give that memory back after a call (`malloc_zone_pressure_relief` on macOS).

9. **Codec choice:** most of WebRTC's share is software VP8 encoding (the headless peer's 14–21%). Preferring hardware H.264 (VideoToolbox, Media Foundation) through `set_codec_preferences`, when the factory reports a hardware encoder, is probably the largest win left for laptops on battery, perhaps 5–10 points. The browser side has to agree, and bitrate behavior differs.

## Smaller things

- `Smoothed::record` in `engine/src/video.rs` does a non-atomic read-modify-write. Two feeds racing only lose a sample, so it's harmless, and `fetch_update` would cost the same.
- Screen capture polls at a fixed 60 Hz (`FRAME_EVERY` in `engine/src/screen/mod.rs`), even when the screen doesn't change. ScreenCaptureKit and Windows Graphics Capture deliver only changed frames, so letting the capturer set the pace would stop those wakeups.
- The room socket (every 10 s) and the friends socket (every 20 s) ping on separate timers. Aligning them on Android would cut radio wakeups.

## Translation on iOS

iOS won't let a sideloaded app map memory executable, so wasmtime can't run Bergamot's ahead-of-time machine code there. Measured on 2026-11-15 on an M-series Mac, zh→en, the 41 lines of `tests/bergamot_reference.json`: wasmtime's Pulley interpreter (after rewriting the module's atomics as plain loads, stores and helper calls, since Pulley has no threads) gave identical output, but took **~390 ms a line (max 670 ms)** against **~9 ms** for the machine code, about 45× slower, and loaded in ~460 ms against ~70 ms. The app translates the other person's live line every time it grows, so that would keep a core busy for as long as they talk. Pulley was dropped. A native C++ build of bergamot-translator replaces it, first on iOS, then on every platform if it's at least as fast as wasmtime there (Phase 5, I0).

## Server

The server is already lean. These are notes rather than findings:

- `sessionUser` runs a SQL lookup on each WebSocket upgrade, but it returns early when there's no token, so only signed-in clients pay for it.
- Each rate limiter keeps its own `setInterval` sweep. One shared sweep would be tidier, not faster.
- The static file handler runs `stat` + `readFile` per request with no cache. Railway's edge caches the immutable assets, so this only matters for `index.html`.
