# Changelog

What changed in each release of the native app, newest first. CI copies a version's section into its GitHub release
(`native-v<version>`), so every released version needs one. How to write it: `native/AGENTS.md`, Releasing.

## 0.2.0 — 2026-11-08

The app comes to Android, and speech can be recognized on the device.

### New

- Android app: calls keep going with the screen off, with a notification to mute or hang up; camera, picture-in-picture and phone layouts; updates itself
- Speech to chat can run on this device: download the speech model (about 240 MB) from the first-launch prompt or Settings, and speech no longer goes to the server
- Settings screen
- Camera conversion time in the stats overlay

### Changed

- Sharper screen sharing: up to 15 Mbps at 60 fps, and the stats overlay shows the codec

## 0.1.0 — 2026-11-08

First release, at parity with the web app.

### New

- Audio and video calls with a browser or another app, direct when possible and through our relay when not
- Setup screen with mic and camera pickers, mic level, portrait mode and "I speak" (English or Chinese)
- Chat with zh ↔ en translation, speech to chat and subtitles
- Screen sharing next to the camera, up to 60 fps, with the computer's sound
- Draggable video tiles, rotate the other person's video, switch camera, stats overlay
- Screenshots with a date stamp
- Installs per user and updates itself from call.danendz.com: quietly on quit, or from the "Update to …" button
