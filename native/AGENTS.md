# Native app (Rust + Slint)

Desktop client for macOS and Windows, and the Android app (Phase 4 in the roadmap) built from the same crates. It talks to the same server over the same `/ws` protocol as the web app, so a native user can call a browser user. The root `AGENTS.md` still applies (git workflow, China, security, roadmap); this file adds what's specific to Rust and Slint.

Current goal: **parity with the web app**. Native-only features (overlay, tray, hotkeys) come after, see `docs/ROADMAP.md`.

## Layout

```
native/
  crates/protocol/   serde mirror of shared/protocol.ts, no other logic
  crates/engine/     everything that isn't UI: controller, signaling, peer, media, speech, stats
  app/               the Slint UI and the bridge from engine state to it
    ui/              *.slint: theme.slint (tokens), one folder per screen, components/ for shared ones
    src/             lib.rs (the app's run()), main.rs (desktop entry), android/ (Android entry), the bridge
    translations/    zh .po, bundled into the binary
  android/           Gradle project: the APK, MainActivity and what only Java can do
  vendor/            third-party crates we patch (webrtc-sys), outside the workspace; see its README
```

- **`protocol` mirrors `shared/protocol.ts` by hand.** The TS file is the source of truth. When it changes: update `shared/protocol.fixtures.ts` with an example of the new shape, run `npm run fixtures`, then update `crates/protocol` until `cargo test -p protocol` passes. Never add a message only on the Rust side.
- **The engine owns state, the UI renders it.** Like `web/src/call/controller.ts`: the UI sends `Command`s and receives state snapshots and events. No call logic in `.slint` files or in Slint callbacks beyond sending a command. The account and friends run apart from the call, in `engine::friends` on their own thread (`/api` and `/ws/user`), like the web's `FriendsSocket` next to `CallController`; the app hands an answered call to the call engine (`app/src/friends.rs`). The session token is kept in its own file in app data, never in `settings.json`, and never logged (`SessionToken`'s `Debug` hides it).
- **Code used by one screen stays in that screen's folder.** Move it to `components/` (Slint) or a shared module only once a second screen needs it.
- **Split a file by what it does once it grows past ~300 lines.**

## Rust

- Edition 2024, toolchain pinned in `rust-toolchain.toml`. Workspace lints in `Cargo.toml` apply to every crate (`[lints] workspace = true`).
- **Nothing from the network may panic.** No `unwrap`/`expect`/indexing on data from the server, the other peer or the OS (clippy warns, CI denies warnings). Parse every message fallibly; an unknown message type is logged and ignored, never fatal. Tests may unwrap.
- **Errors:** `thiserror` enums in `protocol` and `engine`, `anyhow` only in `app`. Log with `tracing`; never log the member `token`, TURN credentials or message text.
- **`unsafe` only in FFI modules**, each block with a `// SAFETY:` comment saying why it holds.
- **Threads:** the engine runs on a tokio runtime off the UI thread. Results reach the UI only through `slint::invoke_from_event_loop` (or a `Weak` handle's `upgrade_in_event_loop`). Never block the UI thread: no network, file or device calls in Slint callbacks.
- **Video frames are dropped, never queued.** Keep the newest frame; a slow consumer must not build latency.
- **Dependencies:** each crate needs a reason a few lines of our own code can't cover. Pin with `Cargo.lock` (committed). CI runs `cargo audit`.

## Slint

- **Tokens live in `ui/theme.slint`** as a `global Theme`, ported from `web/src/ui/tokens.css`: colors, radii, durations (`fast` 150ms, `med` 260ms, `slow` 420ms) and easings (`ease`, `ease-spring` as `cubic-bezier`). No raw colors or durations elsewhere.
- **Animations match the web's** (`fade-in`, `rise`, `pop`, `msg-in` in the web CSS) and use token durations and easings. Animate opacity and position, not layout sizes, so frames stay cheap. Honor reduced motion: `Theme.motion` scales durations to near zero.
- **Glass:** Slint has no backdrop blur, so glass surfaces use the `glass-strong` color with a drop shadow instead.
- **Components are small and take properties in, callbacks out.** No global mutable state besides `Theme` and the app state global the bridge fills.
- **Lists use models** (`VecModel` in Rust, `[T]` in Slint) and are updated in place: append a chat message, set one row's translation. Don't rebuild a whole model on every change.
- **Every user-visible string goes through `@tr("…")`.** English is the source; Chinese lives in `translations/zh/LC_MESSAGES/call-danya.po`, bundled into the binary and picked at runtime from "I speak". Plural forms use `@tr("{n} …" % n)`.
- Renderer is Skia on OpenGL. Video goes through the `VideoSink` trait into a reused `SharedPixelBuffer`; if conversion time (shown in the stats overlay) gets too high, the planned upgrade is `BorrowedOpenGLTexture` with a YUV shader, not a different toolkit.

## Reachable from China

- The binary must not contact anything but our server (`call.danendz.com` in release builds, `http://localhost:8090` in debug builds; `CALL_SERVER` env overrides either) and our coturn. Updates come from our server too (`/updates`, see Distribution). No crash reporters, analytics or crates that phone home.
- Build-time downloads (Skia, libwebrtc prebuilt) are fine. At runtime the app downloads only from our own server (app updates, speech models), never from Hugging Face, GitHub or other third-party hosts, and checks each downloaded file against a SHA-256 built into the binary before using it.

## WebRTC

- **LiveKit's `libwebrtc` crate**, Google's libwebrtc prebuilt. The first build downloads it (~200 MB) into `target/`. On macOS it needs `-ObjC` at link time (`.cargo/config.toml`), or creating the factory crashes. `libwebrtc`'s `webrtc-sys` is patched in from `vendor/` with a hook for our noise suppression: change it only through `vendor/webrtc-sys.patch` (`vendor/README.md` says why and how).
- **Audio uses WebRTC's platform audio device** (`media.rs`): it captures the picked mic and plays the other person through the default output, with WebRTC's echo canceller, noise suppression and gain control, like a browser with the web's constraints. Its blocking calls (starting playout) run on WebRTC's signaling thread, so a peer connection call from the engine can wait on them.
- **The setup screen's mic level** opens the mic separately with `cpal` (`level.rs`), only while that screen is up: WebRTC captures nothing until a call.
- **Video frames reach the screen through `VideoSink`** (`engine/src/video.rs`, the app's `video.rs`): each feed keeps only its newest frame, converted straight into a fresh Slint buffer, and at most one UI hop per feed is pending, so a busy UI skips frames instead of falling behind. Conversion time shows in the stats overlay.
- **Cameras** (`engine/src/camera/`): AVFoundation on macOS, handing the camera's `CVPixelBuffer` to WebRTC without a copy (portrait crops go through I420); Media Foundation on Windows through `nokhwa`, raw NV12/YUY2 only, since its MJPEG decoding would add a third libjpeg; Camera2 on Android, opened by `Camera.kt`, which hands each YUV_420_888 frame to `engine::android_camera::frame` to be cropped (`convert.rs`). Its pixels stay as the sideways sensor gives them and the frame carries the turn that makes it upright: the receiver draws it turned (our self-view too, mirrored by flipping rows), since turning it on the CPU took a third of a phone's core. 720p at 60 fps where the camera has it, else 30; the bitrate cap follows (`peer.rs` `tune_senders`). One video track lives for the app's whole life, so switching or turning the camera on never renegotiates. `CALL_FAKE_CAMERA=1` swaps every camera for a moving test pattern, for testing video where no camera can be used.
- **Speech to chat** (`engine/src/speech/`) opens the mic apart from the call, like the level meter, and runs it through WebRTC's `AudioProcessingModule` with the other person's decoded voice as the echo reference, so their voice from the speakers isn't transcribed as yours. Utterances are cut like the web's recorder and go to a `Recognizer`: streamed to `/ws/speech`, or, once the on-device model is downloaded and picked in Settings, decoded whole at the pause by SenseVoice-Small through `sherpa-onnx` (`local.rs`; static libraries on desktop, two `.so` files the Gradle build copies into the APK on Android). The model is ~240 MB, downloaded from `/updates/` into the app data folder (`model.rs`: resumable, SHA-256 checked); if it doesn't load, speech goes to the server. Android's mic comes in ~20 dB quieter than a desktop one, so the speech path boosts it there (`MIC_GAIN`). With live subtitles on, the live model (`live.rs`, ~275 MB) takes over once it's downloaded: sherpa-onnx's streaming Zipformer decodes while you speak, its text so far goes out as `chat-live` up to 4 times a second, and at the pause a CT-Transformer punctuates it into the chat line that finishes it (`liveId`). The start screen offers it together with the translation model in one download; SenseVoice stays in Settings for when live subtitles are off. `CALL_FAKE_MIC=<file.wav>` loops a 16-bit mono WAV in place of the mic (`say -o hi.wav --data-format=LEI16@16000 "…"` makes one). `cargo run -p engine --example recognize -- <model folder> <file.wav> [zh|en]` times a model on a file (a live model's folder prints its text as it grows); the `join` example recognizes on this machine with `DATA_DIR=<folder>` (and `DOWNLOAD=1` fetches the model first, `LIVE=1` the live one, sent as live text).
- **Live subtitles' translation** (`engine/src/translate/`) runs Firefox's Bergamot engine, the same `vendor/bergamot/bergamot.wasm` the web runs in a Worker. `engine/build.rs` compiles it to machine code for the target with wasmtime (Cranelift is a build dependency only), and `host.rs` runs it with a small embind host standing in for the JavaScript glue: bounds-checked memory access, every object a line makes deleted after it. One direction's model (~55 MB, from `/updates/` like the speech model, through the shared `download.rs`) loads on its own thread while a call with subtitles is up; the app's `translate.rs` asks it for the other language's lines and keeps what it got. The ignored test `tests/translate.rs` checks the host's output line for line against Firefox's engine in Node; run it after touching the host or updating the engine.
- **Screen sharing** (`engine/src/screen/`) goes out in its own stream next to the camera, announced in `media` messages like the web's; their video is routed to the camera or screen feed by stream id (`remote.rs`). macOS uses the ScreenCaptureKit picker; Windows lists screens and windows in the app and captures them with Windows Graphics Capture. Up to 60 fps, scaled to fit 1080p. Its sound (`screen_audio/`) is everything the computer plays except this app, which plays the other person: ScreenCaptureKit on macOS 13+ (asks for Screen Recording permission if the picker didn't), process loopback on Windows; it goes out in the screen's stream. `CALL_FAKE_SCREEN=1` shares a test pattern (the sound is still real).
- **Screenshots** are drawn with Skia (`skia-safe`, pinned to the version Slint's renderer uses, so nothing extra is built) from the frames on screen.
- **macOS needs the app bundle** (`packaging/macos/`) for mic and camera permission prompts; a bare binary borrows the terminal's permissions, which in dev is often "denied", so a dev build sends silence.

## Android

- **The app is the same Rust library**, built as `libcall_danya.so` by `cargo-ndk` from the Gradle project in `android/` (arm64 only, minSdk 29). `MainActivity` extends `NativeActivity`: it hands WebRTC the app context (`nativeInit`, on the Java main thread where WebRTC's classes can be found), then `android_main` (`app/src/android/`) starts Slint's `android-activity` backend and the same `run()` as desktop. Logs go to logcat (`adb logcat -s call-danya`).
- **Platform code** goes in `cfg(target_os = "android")` modules next to the macOS and Windows ones; anything only Java can do (foreground service, permissions, MediaProjection, installing an update) goes in Kotlin in `android/`, kept small. Velopack and the winit window hook are desktop only.
- **WebRTC on Android needs its Java half:** `app/build.rs` exports the prebuilt's `Java_livekit_org_webrtc_*` JNI symbols from our library and copies its `libwebrtc.jar` next to it, where Gradle picks it up.
- **Rust and Kotlin talk through `Shim.kt` and `app/src/android/mod.rs`.** Rust calls Shim's static methods through JNI with the class cached in `nativeInit` (a Rust thread's class loader can't find app classes), and Shim calls back through `Shim.native*`, handled on the UI thread. The ticker in `run()` calls `android::sync`, which starts `CallService` (the foreground service and its notification with mute and hang up) for the call and stops it after. Shim owns the call's audio session: communication mode, audio focus (a phone call mutes the mic until it ends), speaker / earpiece / Bluetooth / wired or USB headset routing (the mic follows the route, so Android has no mic list) and the proximity wake lock. It also asks for the camera (through `MainActivity`, which asks for the mic and camera together at launch) and sets the picture-in-picture shape that `MainActivity` enters on Home or Back during a call; `pip` on the window then shows only the main video.
- **Back never ends the activity:** `MainActivity.finish()` moves the app to the background, so the call goes on. The phone's Back key reaches Slint as `Key.Back`, and only a focused item sees keys: `app.slint` holds focus in a `FocusScope` when nothing else has it, and closing the keyboard or the chat hands focus to a scope rather than clearing it.
- **`Media` keeps an idle peer connection on Android.** WebRTC shuts the audio device down when its last peer connection goes, and LiveKit's proxy never starts it again: the next call crashed in `AudioDeviceBuffer::RegisterAudioCallback`.
- **The window is drawn under the status and navigation bars**, so `app.slint` keeps every screen inside `safe-area-insets` (zero on desktop).
- **Build and run** (needs the SDK with NDK 28.2 and platform 35, `cargo install cargo-ndk`, `rustup target add aarch64-linux-android`, and a JDK, e.g. Android Studio's: `export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"`):

  ```
  cd android && ./gradlew assembleDebug
  adb install -r -g app/build/outputs/apk/debug/app-debug.apk
  adb reverse tcp:8090 tcp:8090   # a debug build talks to the dev server at localhost:8090
  ```

  Where github.com can't be reached, the WebRTC prebuilt download fails: fetch `webrtc-android-arm64-release.zip` from LiveKit's `rust-sdks` release named in `webrtc-sys-build`'s `WEBRTC_TAG` (`gh release download` works when the site doesn't), unzip it and point `LK_CUSTOM_WEBRTC` at the folder for Android builds only.
- **An emulator** (an arm64 API 31 image is closest to HarmonyOS 4.x) checks that the app starts, draws and joins; sound, echo cancellation and background behavior need a real phone.

## Distribution

- **Velopack** (`velopack` crate, `vpk` CLI, same pinned version in `app/Cargo.toml` and `native.yml`) installs the app per user and updates it (`app/src/update.rs`): it checks `{server}/updates` at launch and every 4 hours, downloads in the background (its percent shows on the start screen), and installs on quit or from the start screen's "Update to …" button, never during a call. A dev build isn't installed, so it skips all of this.
- **A release is a version bump** (see Releasing), merged to `main` and then pushed to a `release/<version>` branch. CI builds both OSes and `packaging/release.sh` packs them and uploads to the Railway `releases` bucket, which the server serves at `/updates/*` (`server/updates.ts`), then makes the GitHub release. Feed `releases.{win,osx}.json`, first installs `CallDanya-win-Setup.exe` and `CallDanya-osx-Setup.pkg` (or `-Portable.zip`).
- **Android** (`packaging/android/release.sh`, CI job `release-android` on Linux) builds the APK signed with our release key and uploads `call-danya-<version>.apk`, the same file as `call-danya.apk` for first installs, then `android.json` (version, version code, file, SHA-256). A release build's `Updater.kt` checks it at launch and every 4 hours, downloads a newer APK into the app's cache, checks its SHA-256 and shows the same "Update to …" button, which hands it to the system installer (it asks to confirm, and once to allow installing apps from Call Danya). A debug build doesn't check. On Huawei, installing an APK needs Pure mode off.
- **The Android key can't be replaced:** Android only installs an update signed with the installed app's key, so a new key means uninstalling and installing again on every phone. It lives in the `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` secrets, with an offline backup. A `build/**` push also builds the signed APK as an artifact, so it installs over a released one and updates from then on.
- **Signing costs nothing:** Windows is unsigned (SmartScreen "Run anyway" once; add the install folder, `%LocalAppData%\CallDanya`, to an antivirus's trust list if it flags an update). macOS is signed with our self-signed "Call Danya" certificate (`MACOS_CERT_P12` secret) and the hardened runtime with `packaging/macos/call-danya.entitlements`, so camera, mic and screen recording permissions survive updates; first install is right-click → Open. Replacing the certificate makes macOS ask for those permissions once more.

## Releasing

Do these the same way every time, so releases read alike:

1. From an up-to-date `main`: `git checkout -b bump/<version>`. Pick the version by semver from what changed since the last `native-v*` tag (`git log --oneline --no-merges native-v<last>..main -- native server/updates.ts`): patch for fixes only, minor for anything new. Until 1.0, a breaking change is also minor.
2. Set `version` in `app/Cargo.toml` and run `cargo build` so `Cargo.lock` follows. `packaging/macos/bundle.sh` copies the version into the macOS bundle; nothing else holds it.
3. Add a section at the top of `CHANGELOG.md` (CI fails the PR without one; the GitHub release gets its body from it):

   ```
   ## <version> — <YYYY-MM-DD>

   <One sentence: what this release is about.>

   ### New
   - <What users can now do, in a few words>

   ### Fixed
   - <What was broken, said as what works now>

   ### Changed
   - <Behavior that's different, when it isn't new or a fix>
   ```

   Leave out empty headings. Write for the people using the app, not for developers: no crate names, file paths or commit types. Group the commits since the last tag by feature, one line each; leave out CI, docs and refactors that change nothing for the user.
4. Commit as `chore(native): release <version>` and open the PR titled the same, with the new changelog section as its body.
5. After the merge, publish it from an up-to-date `main`: `git push origin main:release/<version>`. The `native` workflow on that branch runs `version` (fails unless the branch matches `app/Cargo.toml`'s version and the commit is on `main`) → `release` (macOS, Windows) and `release-android` → `github-release`. Merging to `main` alone publishes nothing. If one OS fails, rerun the failed jobs: both release scripts skip a channel that already has the version, and `github-release` only adds what's missing.

## Before saying it's done

From `native/`:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

CI runs fmt, clippy and tests on macOS for every PR once it's out of draft (they don't build on Linux: `webrtc-sys` needs clang 21+ there), and Android clippy on Linux, where the NDK brings its own clang. The macOS and Windows release builds only build: they make the installable artifacts and are the only compile of Windows-only code (which no CI lints, so keep it clippy-clean by eye), and still take a while, so they run only on a push to a `build/**` branch (or `gh workflow run native.yml --ref build/<name>`) and for a release on a `release/<version>` branch (see Releasing). Nothing runs on `main` except `native-cache.yml`, which saves the Rust caches these jobs start from when `Cargo.lock` changes. Push to a `build/**` branch only when the user asks for a build; don't offer to: Actions minutes are limited and a build costs a lot of them.

When `shared/` changed, also `npm run fixtures -- --check`, `npm run typecheck` and `npm run build` from the root. Check UI changes by running the app on a single instance (start, setup, waiting screens) against a local server: `PORT=8090 npm run dev:server` in the root (8080 is often taken), then `cargo run` here (a debug build uses `http://localhost:8090` unless `CALL_SERVER` says otherwise). Leave real two-person calls to the user.

Drive the running app by accessible label, never with synthetic mouse clicks, so the user's mouse and focus are left alone. Both drivers take `tree`, `press <label>`, `set <label> <text>` and `shot <file.png>`; labels are in the app's current language, and an element neither can find needs an `accessible-label`, which it should have anyway.

- `python3 dev/ui.py` goes through Slint's embedded MCP server, so it also works with the screen locked and on Windows. It needs a dev build made for it: `SLINT_EMIT_DEBUG_INFO=1 cargo build -p call-danya --features dev-mcp`, run with `SLINT_MCP_PORT=8765`. Prefer it.
- `swift dev/ui.swift` (macOS) uses the Accessibility API on any build, but needs an unlocked screen and Accessibility and Screen Recording permission for the terminal.

For the other side of a call, `cargo run -p engine --example join -- http://localhost:8090 <room> [seconds]` joins headless and logs its state, the video it receives and the chat (`CALL_FAKE_CAMERA=1` gives it a camera, `SAY="…"` posts a message, `TRANSCRIBE=1` with `CALL_FAKE_MIC` speaks, `SHARE=1` with `CALL_FAKE_SCREEN` shares). Two of them, or one against the app, check signaling and negotiation; whether sound really goes both ways is still for the user to hear.
