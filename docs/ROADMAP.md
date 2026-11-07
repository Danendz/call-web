# Roadmap

Where the project is going: make the web call hold up better on the China ↔ Uzbekistan link, then build a native desktop app in Rust with [Slint](https://slint.dev) for the things a browser can't do (game overlay, notifications, global hotkeys, deep encoder and network control).

The web app stays. The native app is a second client on the same server and the same `/ws` protocol, so a native user can call a browser user. Call-quality work goes into both clients. Overlay and notification features are native only, with no browser workarounds.

**Status:** `[x]` done, `[ ]` not started, `[~]` partly done (the note says what's left).
**Where:** **Web + Native** is built in both clients. **Native only** is built only in the native app.

When a feature here is finished, mark it `[x]` in the same PR that ships it.

## Shipped

- [x] 1-on-1 video call with chat, through our own coturn (`turn.danendz.com`), direct P2P preferred with relay fallback
- [x] Setup screen with device preview and permission help
- [x] Speech-to-text and zh ↔ en chat translation, streamed while speaking, with subtitles
- [x] Stats overlay (path, RTT, loss, bitrate, what limits the video)
- [x] Rotate the other person's video, switch camera
- [x] Call timer and screenshot with date stamp
- [x] Screen share alongside the camera, draggable tiles

## Phase 1 — Call quality (Web + Native)

Ordered by value per hour of work. The native app gets the same behavior from libwebrtc in Phase 2.

- [ ] **Opus RED.** Put `audio/red` before Opus with `setCodecPreferences()`, so each audio packet also carries the previous one and a single lost packet leaves no gap. Falls back to plain Opus if the other side lacks it.
- [ ] **Noise suppression with RNNoise or DeepFilterNet** in an `AudioWorklet`, between the mic and the peer connection. Clearer voice over fans and keyboard while gaming. The WASM is shipped in the bundle (no CDN); it needs `'wasm-unsafe-eval'` in the CSP, which has to be approved on purpose. Native: run DeepFilterNet directly in Rust.
- [~] **Encoder settings.** Done: bitrate caps, `contentHint: 'detail'` for the screen, `degradationPreference`, high priority for the mic (`peer.ts` `tuneSenders`). Left: `scalabilityMode: 'L1T3'` so the stream survives loss better, and choosing the codec per device (H.264 for hardware encoding on weak machines, VP9 where the CPU allows).
- [ ] **Self-hosted speech recognition.** SenseVoice-small through `sherpa-onnx` (`sherpa-onnx-node`) in the server: strong Mandarin, mixed zh+en, punctuation, far fewer made-up lines than Whisper, fast on Railway's CPUs and no quota. Groq Whisper stays as the fallback (or for English). Costs ~0.5 GB RAM and a ~230 MB model in the image, served from our own build, not fetched at runtime.
- [ ] **FSR-style upscaling shader** on the receiving side (EASU + RCAS in WebGL). Sharper picture when the bitrate drops. Polish, not a fix for low bitrate.
- [ ] **WebCodecs with custom FEC and retransmission** over an unreliable data channel. Only if the items above aren't enough: it means writing our own jitter buffer, A/V sync and congestion control. The native app (Phase 3) is the better place for this.

## Phase 2 — Native app (Rust + Slint)

Target Windows and macOS first. Lives in `native/` as a Cargo workspace in this repo, with its conventions in `native/AGENTS.md`. The server stays as it is. M0–M4 reach parity with the web app; nothing native-only goes in before that.

- [x] **M0 — Skeleton.** Slint window (Skia renderer) with the web's tokens, start screen (new room, join by link), setup screen with "I speak", waiting screen, room full and call ended screens, `/ws` join with heartbeat and resume. English and Chinese UI from bundled `.po`. Protocol types written by hand in `crates/protocol` and checked against examples in `shared/protocol.fixtures.ts`. CI builds macOS and Windows (unsigned) on every PR.
- [~] **M1 — Audio call with a browser peer.** WebRTC through Google's libwebrtc via LiveKit's Rust bindings (`libwebrtc` / `webrtc-sys`, prebuilt binaries). Not `webrtc-rs`: it has no capture, codecs or echo cancellation and weaker congestion control. TURN credentials come in `joined`. Start with a spike that proves mic capture, playback and echo cancellation on both OSes before building on it. Device pickers and mic level on the setup screen. Built: platform audio device (echo cancellation, noise suppression, gain control), perfect negotiation with a browser, TURN from `joined`, mute, call timer, mic picker and level, macOS app bundle for permissions. Left: the user hearing a real call both ways on macOS and Windows.
- [ ] **M2 — Camera video** both ways, rendered in Slint: I420 converted to RGBA on the CPU behind a `VideoSink`, conversion time in the stats overlay; Slint's `BorrowedOpenGLTexture` with a YUV shader if that proves too slow. Hardware encoders (Media Foundation, VideoToolbox) where available. Rotate and switch camera, stats overlay, relay → direct upgrade.
- [ ] **M3 — Chat, speech-to-text, translation and subtitles**, using the server's existing endpoints and the member `token`. Speech goes through `/ws/speech` behind a `Recognizer` trait. Screenshot with date stamp.
- [ ] **M4 — Screen share.** Windows Graphics Capture and ScreenCaptureKit, up to 60 fps, alongside the camera. Draggable tiles, auto-hiding controls and a final animation pass: parity with the web app.
- [ ] **M5 — Overlay, notifications and desktop features.** See Phase 3.
- [ ] **M6 — Distribution.** Installers, code signing (Windows certificate, Apple notarization), updates from our own server (GitHub can be slow or blocked in China), no Google services in the binary.

## Phase 3 — Native only

### Don't miss messages while gaming

During screen sharing she's in a game and can't see the chat, so new messages have to reach her over it.

| Feature | Notes |
|---|---|
| [ ] Game overlay window | Transparent, always on top, click-through. Shows chat messages and subtitles over windowed and borderless games. |
| [ ] Sound on new message | Also covers exclusive-fullscreen games, where the overlay can't show. |
| [ ] Read new messages aloud | Optional toggle, offline OS voices. |
| [ ] System notification on new message | Local OS notification, no push service (push goes through Google). Focus modes may hide it in fullscreen games. |
| [ ] Tray icon | Unread count, quick mute, start with the system. |
| [ ] Global hotkeys | Push-to-talk, mute, show/hide overlay, reply to chat without leaving the game. |

### Media and network

| Feature | Notes |
|---|---|
| [ ] On-device speech recognition | Apple SpeechAnalyzer on macOS 26+ (on-device, punctuated Mandarin), `sherpa-onnx` streaming zh+en elsewhere, the server as fallback. Free, live partial subtitles while speaking, works offline. Plugs in behind the `Recognizer` trait from M3. |
| [ ] Share one app's audio | Only the game's sound, not Discord or music: Windows process loopback capture, ScreenCaptureKit per-app audio. The browser can only share a tab's or the whole system's audio. |
| [ ] Native super-resolution | VideoToolbox super-resolution scaler on macOS, NVIDIA RTX Video Super Resolution on Windows. Stronger than the web shader. |
| [ ] Custom FEC, retransmission and congestion control | libwebrtc field trials (FlexFEC, more RED redundancy) or changes to the source, tuned for the China link. |
| [ ] Fine-grained encoder tuning | Per-encoder options (x264, libvpx, NVENC), region-of-interest encoding (more bits on the face), keyframe and rate-control control. |

## Not planned

- **Overlay and notification workarounds in the web app** (Document Picture-in-Picture chat window, browser notifications, message sounds, tab badges). These belong to the native app.
- **Overlay inside exclusive-fullscreen games** (drawing into the game's DirectX/Vulkan output, like Discord or Steam). Hard to build, and anti-cheat systems (Vanguard, EasyAntiCheat) can flag it and ban the account. Borderless windowed mode plus the overlay window covers most games.
