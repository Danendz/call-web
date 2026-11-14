# Roadmap

Where the project is going: make the web call hold up better on calls into mainland China, then build a native desktop app in Rust with [Slint](https://slint.dev) for the things a browser can't do (game overlay, notifications, global hotkeys, deep encoder and network control).

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
- [x] Switch mic, camera and speaker during a call from the control pill (bottom sheets on narrow screens), mute the speaker with the other person seeing it, speech to chat in the mic menu (Web + Native; Android picks the route, a headset bringing its own mic)
- [x] Settings as a sheet over the screen (from the bottom on narrow screens, a side panel on the right otherwise), opened from ⋯ in a call and from a gear on the first screen (setup on the web, start on native): connection stats (remembered), "I speak", and on native the speech model (Web + Native)
- [~] Emoji reactions (Web + Native): the pill's emoji panel (8 quick ones and a grid of ~140) sends one, relayed and never stored, which floats up over both sides' video and shows in the chat, a run of the same one growing in place (❤️ ×5); the same one tapped fast becomes a combo (×N and a burst), and every fifth ❤️, 🎉 or 🔥 fills the screen with hearts, confetti or flames. Android vibrates on the other person's. Emoji in typed messages come from the system keyboard. Left: checking that the native app draws color emoji on macOS, Windows and Android.
- [x] Overlay chat (Web + Native): see-through bubbles over the video in the lower left (theirs dark on the left, yours accent-tinted on the right), the oldest fading out at the top and the rest a scroll back, with spoken lines and live text in the same list in place of separate subtitles. The pill's chat button hides it all and counts unread messages meanwhile; the message box fades with the controls unless it's in use, and on a phone it sits above the keyboard. On a phone or a short screen the controls are a rail on the right above the message box (mic, camera, emoji, ⋯ holding the chat toggle), hang up alone in the top corner away from the send button; on wider screens the message box is level with the pill.

## Phase 1 — Call quality (Web + Native)

Ordered by value per hour of work. The native app gets the same behavior from libwebrtc in Phase 2.

- [x] **Opus RED.** Put `audio/red` before Opus with `setCodecPreferences()`, so each audio packet also carries the previous one and a single lost packet leaves no gap. Falls back to plain Opus if the other side lacks it.
- [x] **Noise suppression with DeepFilterNet3** in an `AudioWorklet`, between the mic and the peer connection. Clearer voice over fans and keyboard while gaming. The WASM is shipped in the bundle (no CDN), with `'wasm-unsafe-eval'` in the CSP. Native: the same Rust crate (`native/crates/denoise`) inside WebRTC's audio processing, through a fork of `webrtc-sys`. A switch in Settings, off by default on Android.
- [~] **Encoder settings.** Done: bitrate caps, `contentHint: 'detail'` for the screen, `degradationPreference`, high priority for the mic (`peer.ts` `tuneSenders`). Left: `scalabilityMode: 'L1T3'` so the stream survives loss better.
- [x] **Video codec per device.** H.264 first where the device encodes it in hardware (VideoToolbox, MediaCodec, a browser whose `mediaCapabilities` says power efficient), left out where it doesn't, so nobody runs a software H.264 encoder; VP8 otherwise. Settings has Auto / H.264 / VP8, applied mid-call with one renegotiation; VP8 moves the whole call to VP8, for when H.264 misbehaves. Since each side sends what the other side's description lists first, a codec left out is also removed from the other side's offer before it's applied (`codec.rs`, `codec.ts`).
- [ ] **Self-hosted speech recognition.** SenseVoice-small through `sherpa-onnx` (`sherpa-onnx-node`) in the server: strong Mandarin, mixed zh+en, punctuation, far fewer made-up lines than Whisper, fast on Railway's CPUs and no quota. Groq Whisper stays as the fallback (or for English). Costs ~0.5 GB RAM and a ~230 MB model in the image, served from our own build, not fetched at runtime.
- [ ] **FSR-style upscaling shader** on the receiving side (EASU + RCAS in WebGL). Sharper picture when the bitrate drops. Polish, not a fix for low bitrate.
- [~] **Live subtitles with on-device translation** (Web + Native). What the other person is saying shows in the chat as they say it, translated as they speak; a "Live subtitles" switch in Settings, on by default. Done: the live line at the end of the overlay chat (the finished line takes its place, the server's translation crossfading in), a `chat-live` message the server relays without storing, and translation on the device with Firefox's Bergamot (native through wasmtime, web in a Worker; one direction's ~55 MB model, offered when subtitles are on and again when the language changes, and in Settings). Native also recognizes your speech as you say it (sherpa-onnx's streaming Zipformer, punctuated by a CT-Transformer once the line ends) and sends it as `chat-live`, with one ~330 MB download for both models offered on the start screen in place of the SenseVoice one. Left: measuring memory and speed on an Android phone (on desktop the speech models hold ~610 MB, most of it the punctuation model, plus ~350 MB for translation).
- [ ] **Web sends live text** (Web). Streaming recognition in the browser through sherpa-onnx's WASM build (~260 MB), so web speech also shows while it's being said.
- [ ] **Keep spoken lines through a reconnect** (Web + Native). A finished line said while the room socket is down or reconnecting is dropped (`native/crates/engine/src/signaling.rs` outgoing queue, `web/src/call/signaling.ts` `send`), though its live text may already be on the other person's screen. Queue spoken lines and send them once the socket is back.
- [ ] **Chat history past 200 lines on rejoin** (Web + Native). A rejoin or resume replaces the chat with the server's copy, capped at `MAX_CHAT_HISTORY = 200` (`server/rooms.ts`); with speech to chat a long call passes that and older lines vanish. Merge with what the client holds, or raise the cap.
- [ ] **WebCodecs with custom FEC and retransmission** over an unreliable data channel. Only if the items above aren't enough: it means writing our own jitter buffer, A/V sync and congestion control. The native app (Phase 3) is the better place for this.

## Phase 2 — Native app (Rust + Slint)

Target Windows and macOS first. Lives in `native/` as a Cargo workspace in this repo, with its conventions in `native/AGENTS.md`. The server stays as it is. M0–M4 reach parity with the web app; nothing native-only goes in before that.

- [x] **M0 — Skeleton.** Slint window (Skia renderer) with the web's tokens, start screen (new room, join by link), setup screen with "I speak", waiting screen, room full and call ended screens, `/ws` join with heartbeat and resume. English and Chinese UI from bundled `.po`. Protocol types written by hand in `crates/protocol` and checked against examples in `shared/protocol.fixtures.ts`. CI checks every PR on macOS and builds macOS and Windows (unsigned) on `build/**` branches.
- [x] **M1 — Audio call with a browser peer.** WebRTC through Google's libwebrtc via LiveKit's Rust bindings (`libwebrtc` / `webrtc-sys`, prebuilt binaries). Not `webrtc-rs`: it has no capture, codecs or echo cancellation and weaker congestion control. TURN credentials come in `joined`. Start with a spike that proves mic capture, playback and echo cancellation on both OSes before building on it. Device pickers and mic level on the setup screen. Built: platform audio device (echo cancellation, noise suppression, gain control), perfect negotiation with a browser, TURN from `joined`, mute, call timer, mic picker and level, macOS app bundle for permissions.
- [x] **M2 — Camera video** both ways, rendered in Slint: I420 converted to RGBA on the CPU behind a `VideoSink`, conversion time in the stats overlay; Slint's `BorrowedOpenGLTexture` with a YUV shader if that proves too slow. Hardware encoders (Media Foundation, VideoToolbox) where available. Rotate and switch camera, stats overlay, relay → direct upgrade. Built: camera both ways (AVFoundation zero-copy on macOS, Media Foundation on Windows), self tile, camera off, picker and switch, portrait, rotate, stats overlay with conversion time, relay → direct upgrade, force relay. 60 fps where the camera has it (an iPhone as Continuity Camera; built-in Mac cameras stop at 30), with a higher bitrate cap than at 30.
- [x] **M3 — Chat, speech-to-text, translation and subtitles**, using the server's existing endpoints and the member `token`. Speech goes through `/ws/speech` behind a `Recognizer` trait. Screenshot with date stamp. Built: chat panel with translations and unread badge, speech to chat over `/ws/speech` with echo cancellation against the other person's voice, subtitles, paused/unavailable notices, screenshot with date stamp and the other side's notice.
- [x] **M4 — Screen share.** Windows Graphics Capture and ScreenCaptureKit, up to 60 fps, alongside the camera. Draggable tiles, auto-hiding controls and a final animation pass: parity with the web app. Built: sharing alongside the camera both ways (ScreenCaptureKit picker on macOS, an in-app picker on Windows), the web's bitrate split, tiles you drag to a corner (remembered) or tap onto the stage, auto-hiding controls, popup and menu animations. Up to 60 fps (WebRTC's capturers: ScreenCaptureKit, and Windows Graphics Capture), scaled to fit 1080p, 15 Mbps ceiling that congestion control settles under; short of bandwidth, a balance of frame rate and resolution (a playing video stays watchable), on the web too, which now also captures at up to 60 fps. The stats overlay shows each side's codec and encoder. The computer's sound goes with the share, without the call's own (ScreenCaptureKit on macOS 13+, process loopback on Windows).
- [ ] **M5 — Overlay, notifications and desktop features.** See Phase 3.
- [x] **M6 — Distribution.** Installers, signing, updates from our own server (GitHub can be slow or blocked in China), no Google services in the binary. Built: Velopack per-user installers and delta updates, from the Railway `releases` bucket through `call.danendz.com/updates`; a version bump merged to `main` and pushed to `release/<version>` publishes a release; updates download quietly and install on quit or from an "Update" button on the start screen. Signing at no cost: Windows unsigned, macOS with a self-signed certificate so permissions survive updates (paid signing only if the app ever goes beyond personal use). No Google service hosts in the release binary (checked with `strings`).

## Phase 2b — Accounts, friends and call requests (Web + Native)

Accounts live next to anonymous room links, which keep working. Stored in Postgres on Railway; rings, presence and rooms stay in memory.

- [x] **Server.** Invite-only accounts (username and scrypt password, recovery code shown once), sessions as a cookie on the web and a bearer token on native, friend requests, block, delete account. `/ws/user` carries friends with presence and rings every open client of a friend for 45 s; unanswered or offline becomes a missed call. An answered call gets a private room only the two accounts can join; one who left can rejoin from the Rooms list while the other is still inside. Anonymous link rooms get tighter limits than a signed-in user's: sockets, rooms, chat and speech rate, an hour of audio a day per IP, a 2 h call.
- [x] **Web.** Sign in, create account, recovery, account sheet (invite codes, password, blocked, delete), friends page with presence, requests, missed calls and Rooms, incoming and outgoing call overlay with a ring tone.
- [~] **Native desktop.** Built: sign in, create account and recovery, friends on the start screen with presence, requests, missed calls and Rooms, the call overlay, account controls in Settings, and the session token kept in its own file in app data (sent on the room socket for private rooms). Left: a ring tone for incoming calls.
- [~] **Android.** Built: the same screens fit a phone (the call card stacks its buttons, Back closes the sign-in dialog). Left: a ring only reaches the app while it's open; ringing a phone in the background needs a notification from a foreground service, since no push service works in China without a vendor SDK (with A3).

## Phase 3 — Native only

### Don't miss messages while gaming

During screen sharing the other person may be in a full-screen game and can't see the chat, so new messages have to reach them over it.

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
| [ ] Hardware H.264 on Windows | LiveKit's WebRTC has no hardware encoder or decoder on Windows (NVENC is Linux only there), so Auto keeps Windows on VP8. A Media Foundation H.264 encoder and decoder (Intel Quick Sync, AMD, NVIDIA through one API) added to the Windows factories through `vendor/webrtc-sys.patch`, fed from CPU frames like now; then Auto picks H.264 there too. |
| [~] On-device speech recognition | SenseVoice-Small (int8) through `sherpa-onnx` on macOS, Windows and Android, one model for Mandarin, English and the two mixed, punctuated, told the language picked in "I speak". Whole utterances, cut by the same pause detection as the server path, decoded when the pause comes: a fraction of a second on a phone. The model (240 MB) isn't in the installer: Settings downloads, cancels or deletes it and switches between this device and the server. With live subtitles on, the streaming model from **Live subtitles** recognizes instead, decoding while you speak, and the start screen offers that one. It comes from our server (the `releases` bucket, `/updates`), resumable, checked against a SHA-256 built into the binary, and kept in the app data folder; until it's ready, or if it fails to load, speech goes to the server. Free, no quota, works offline. Plugs in behind the `Recognizer` trait from M3. Left: Apple SpeechAnalyzer on macOS 26+ if it proves better. |
| [ ] Share one app's audio | Only the game's sound, not Discord or music: Windows process loopback capture, ScreenCaptureKit per-app audio. The browser can only share a tab's or the whole system's audio. |
| [ ] Native super-resolution | VideoToolbox super-resolution scaler on macOS, NVIDIA RTX Video Super Resolution on Windows. Stronger than the web shader. |
| [ ] Custom FEC, retransmission and congestion control | libwebrtc field trials (FlexFEC, more RED redundancy) or changes to the source, tuned for the China link. |
| [ ] Fine-grained encoder tuning | Per-encoder options (x264, libvpx, NVENC), region-of-interest encoding (more bits on the face), keyframe and rate-control control. |

## Phase 4 — Android (Native only)

Built for calls, sharing a mobile game, and speech and translation from a phone. The target includes Huawei phones on HarmonyOS 4.x, which run Android apps (about API 31) and have no Google services. HarmonyOS NEXT (5 and later) can't install APKs at all: on such a phone the web app in its browser stays the fallback.

Why an app and not the mobile web: Chrome on Android can't share the screen; a browser tab doesn't keep a call alive in the background, and Chinese phones (Xiaomi, Huawei, Oppo, Vivo) kill background work aggressively; Chinese phones rarely have Chrome, and links opened in WeChat land in its built-in browser, with weak WebRTC, while an APK ships its own libwebrtc.

Same `native/` workspace and the same Slint screens, with phone layouts: the `app` crate is a library with a desktop `main.rs` and an `android_main` (Slint's `android-activity` backend), `protocol` and the engine's core (controller, signaling, peer, speech) build for Android as they are, LiveKit's `libwebrtc` supports Android (`initialize_android_context` with the JavaVM and app context, plus its `libwebrtc.jar` in the APK), and `cpal` has AAudio. Android code goes in `cfg(target_os = "android")` modules next to the macOS and Windows ones. What only Java can do (foreground service and its notification, permissions, MediaProjection, installing an update) goes in a small Kotlin shim in `native/android/`, a Gradle project that builds the Rust library with `cargo-ndk`. arm64 only.

- [x] **A0 — Spike.** The engine builds for `aarch64-linux-android`, the Slint app joins a room on a phone and holds an audio call with a browser peer, with WebRTC's platform audio and hardware echo cancellation.
- [x] **A1 — Audio call that survives the background.** Foreground service (types microphone and camera) with a call notification carrying mute and hang up, runtime permissions, earpiece / speaker / Bluetooth routing with the proximity sensor, pause for a phone call. Speech to chat, translation and subtitles over `/ws/speech` as on desktop. A first-run guide to the vendor's battery and app launch settings (on Huawei: App launch → Manage manually), which no API can change. The service also claims the camera type when the camera is allowed, so video keeps going in the background.
- [x] **A2 — Camera and phone layouts.** Camera through Camera2 (in Kotlin, frames handed to Rust, which crops and turns them upright) into WebRTC, front camera first, portrait video by default; hardware encoding through WebRTC's MediaCodec factory. Portrait layouts for every screen, touch-sized controls, chat above the on-screen keyboard, picture-in-picture while in another app (on Home, or on Back during a call).
- [ ] **A4 — Screen share.** MediaProjection with its own foreground service type, and only the game's sound through audio playback capture (Android 10+; a game can opt out of it).
- [x] **A5 — Distribution.** Released with the desktop app, same version and the same `release/<version>` push: CI builds and signs the APK on Linux and uploads `call-danya-<version>.apk` with an `android.json` (version, SHA-256) to the `releases` bucket, and attaches the APK to the GitHub release. The app checks `/updates/android.json`, downloads the APK, checks its SHA-256 and hands it to the system installer. The signing keystore lives in GitHub secrets with an offline backup: an APK signed with another key can't update the installed one. No Google Play, no Chinese app stores (they need ICP filing and software copyright registration), no Google Play services in the APK. On Huawei, installing needs Pure mode off. CI also builds a signed APK on `build/**` and runs Android clippy on PRs; the in-app updater is `Updater.kt`.
- [ ] **A3 — Don't miss messages.** After A5. Local notification and sound on new messages (no push: FCM is blocked in China and missing on many Chinese phones), read aloud with the system's text-to-speech, chat and subtitles in a bubble over other apps ("display over other apps" permission) for mobile games.

Phase 3's on-device speech recognition (`sherpa-onnx`) is built for Android too; FEC / encoder tuning carry over to Android; super-resolution doesn't (no system API, small screen).

## Not planned

- **Overlay and notification workarounds in the web app** (Document Picture-in-Picture chat window, browser notifications, message sounds, tab badges). These belong to the native app.
- **Overlay inside exclusive-fullscreen games** (drawing into the game's DirectX/Vulkan output, like Discord or Steam). Hard to build, and anti-cheat systems (Vanguard, EasyAntiCheat) can flag it and ban the account. Borderless windowed mode plus the overlay window covers most games.
