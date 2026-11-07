# Native app (Rust + Slint)

Desktop client for macOS and Windows. It talks to the same server over the same `/ws` protocol as the web app, so a native user can call a browser user. The root `AGENTS.md` still applies (git workflow, China, security, roadmap); this file adds what's specific to Rust and Slint.

Current goal: **parity with the web app**. Native-only features (overlay, tray, hotkeys) come after, see `docs/ROADMAP.md`.

## Layout

```
native/
  crates/protocol/   serde mirror of shared/protocol.ts, no other logic
  crates/engine/     everything that isn't UI: controller, signaling, peer, media, speech, stats
  app/               the Slint UI and the bridge from engine state to it
    ui/              *.slint: theme.slint (tokens), one folder per screen, components/ for shared ones
    src/             main.rs and the bridge
    translations/    zh .po, bundled into the binary
```

- **`protocol` mirrors `shared/protocol.ts` by hand.** The TS file is the source of truth. When it changes: update `shared/protocol.fixtures.ts` with an example of the new shape, run `npm run fixtures`, then update `crates/protocol` until `cargo test -p protocol` passes. Never add a message only on the Rust side.
- **The engine owns state, the UI renders it.** Like `web/src/call/controller.ts`: the UI sends `Command`s and receives state snapshots and events. No call logic in `.slint` files or in Slint callbacks beyond sending a command.
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

- The binary must not contact anything but our server (`call.danendz.com`, default; `CALL_SERVER` env overrides it for development) and our coturn. No crash reporters, analytics, update checks or crates that phone home.
- Build-time downloads (Skia, libwebrtc prebuilt) are fine; nothing is downloaded at runtime.

## Before saying it's done

From `native/`:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

When `shared/` changed, also `npm run fixtures -- --check`, `npm run typecheck` and `npm run build` from the root. Check UI changes by running the app on a single instance (start, setup, waiting screens) against a local server: `PORT=8090 npm run dev:server` in the root (8080 is often taken), then `CALL_SERVER=http://localhost:8090 cargo run` here. Leave real two-person calls to the user.

Drive the running app with `swift dev/ui.swift` (macOS), not synthetic mouse clicks: it presses buttons by their accessible label through the Accessibility API and screenshots the window even behind others, so the user's mouse and focus are left alone. `tree` lists what's there, `press <label>`, `set <label> <text>`, `shot <file.png>`. Labels are in the app's current language. An element it can't find usually needs an `accessible-label`, which it should have anyway.
