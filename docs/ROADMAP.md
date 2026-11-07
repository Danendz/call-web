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
- [ ] **FSR-style upscaling shader** on the receiving side (EASU + RCAS in WebGL). Sharper picture when the bitrate drops. Polish, not a fix for low bitrate.
- [ ] **WebCodecs with custom FEC and retransmission** over an unreliable data channel. Only if the items above aren't enough: it means writing our own jitter buffer, A/V sync and congestion control. The native app (Phase 3) is the better place for this.

## Phase 2 — Native app (Rust + Slint)

Target Windows and macOS first. Lives in `native/` as a Cargo workspace in this repo. The server stays as it is.

- [ ] **M0 — Skeleton.** Slint window, join a room over `/ws` and show the waiting screen. The Rust types mirror `shared/protocol.ts` with serde; decide early whether to keep them in sync by hand or generate them (e.g. from a JSON Schema) and add a check that they match.
- [ ] **M1 — Audio call with a browser peer.** WebRTC through Google's libwebrtc via LiveKit's Rust bindings (`libwebrtc` / `webrtc-sys`, prebuilt binaries). Not `webrtc-rs`: it has no capture, codecs or echo cancellation and weaker congestion control. TURN credentials from our `/api`.
- [ ] **M2 — Camera video** both ways, rendered in Slint. Hardware encoders (Media Foundation, VideoToolbox) where available.
- [ ] **M3 — Chat, speech-to-text, translation and subtitles**, using the server's existing endpoints and the member `token`.
- [ ] **M4 — Screen share.** Windows Graphics Capture and ScreenCaptureKit, up to 60 fps, alongside the camera.
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
| [ ] Share one app's audio | Only the game's sound, not Discord or music: Windows process loopback capture, ScreenCaptureKit per-app audio. The browser can only share a tab's or the whole system's audio. |
| [ ] Native super-resolution | VideoToolbox super-resolution scaler on macOS, NVIDIA RTX Video Super Resolution on Windows. Stronger than the web shader. |
| [ ] Custom FEC, retransmission and congestion control | libwebrtc field trials (FlexFEC, more RED redundancy) or changes to the source, tuned for the China link. |
| [ ] Fine-grained encoder tuning | Per-encoder options (x264, libvpx, NVENC), region-of-interest encoding (more bits on the face), keyframe and rate-control control. |

## Not planned

- **Overlay and notification workarounds in the web app** (Document Picture-in-Picture chat window, browser notifications, message sounds, tab badges). These belong to the native app.
- **Overlay inside exclusive-fullscreen games** (drawing into the game's DirectX/Vulkan output, like Discord or Steam). Hard to build, and anti-cheat systems (Vanguard, EasyAntiCheat) can flag it and ban the account. Borderless windowed mode plus the overlay window covers most games.
