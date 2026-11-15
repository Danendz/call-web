# Changelog

What changed in each release of the native app, newest first. CI copies a version's section into its GitHub release
(`native-v<version>`), so every released version needs one. How to write it: `native/AGENTS.md`, Releasing.

## 0.5.0 — 2026-11-15

The app comes to iPhone, gets its own icon, and video holds up better on weak connections.

### New

- iPhone: the app installs through AltStore or SideStore, with calls on the lock screen through CallKit, live subtitles translated on the phone, and updates offered in the app
- An app icon on every platform
- The other person's video is sharpened when it's drawn bigger than it arrives (Settings → Video, "Sharpen video")
- Video error correction is on by default, so a lossy connection freezes the video far less
- Missed calls show on the caller's row in Friends, friends sort by missed calls and who's online, and ⋯ removes or blocks a friend
- Settings is split into Audio, Video, Speech, Account and Debug pages
- Settings' Debug page can record each call's stats to a file
- Dialogs, lists and screens animate in and out

### Fixed

- A room that can't be made now says why (your room limit, or the server being busy) instead of "Room is full"
- Android: a call refused right at the start no longer closes the app
- Settings rows no longer spill over their edges, and names sit in the middle of their options

### Changed

- Android draws with Vulkan and converts video on the graphics card, as desktop does; OpenGL stays as the fallback

## 0.4.0 — 2026-11-14

Sharper, smoother video with hardware H.264 on Windows, and better live subtitles.

### New

- Windows: video and screen share are encoded and decoded on the graphics card, so screen share holds 60 fps
- Pick the screen share quality in Settings: Balanced, High, or your own bitrate
- Pick the video codec in Settings (Auto, H.264 or VP8); Auto uses H.264 where the device has a hardware encoder
- Live subtitles work with "Recognize on this device" off too, recognized on the server
- Pick the graphics backend in Settings (Modern or OpenGL); the app switches to OpenGL by itself if Modern doesn't start

### Fixed

- A lost frame no longer freezes the video until the next full picture arrives
- Changing the whole screen during a share no longer freezes it for a moment
- Resizing the window during a call no longer crashes the app
- Rejoining a call on desktop sends your mic and plays the other person again
- A browser joining after the app connects instead of staying on "Connecting"

### Changed

- Live subtitles use the same speech model as speech to chat, end lines at each pause and keep the start of the next one; the older live model is deleted and the new one downloaded in its place
- Less CPU and memory: video is converted on the graphics card and skipped while hidden, and speech models give their memory back after a call

## 0.3.2 — 2026-11-13

Chat now floats over the video, with reactions and subtitles in the same place.

### New

- Pick any emoji as a reaction: the controls hold 8 quick reactions and a full emoji grid
- Phones and short screens: call controls sit in a rail on the right, with hang up alone in the top corner

### Changed

- Chat floats over the video in see-through bubbles, the other person's on the left and yours on the right, older ones fading out at the top
- Spoken lines and live subtitles show in the chat instead of a separate subtitles strip
- Reactions show in the chat, and sending the same one again grows its count in place (❤️ ×5)
- The message box sits level with the call controls

## 0.3.1 — 2026-11-13

Emoji reactions during a call, and emoji in chat.

### New

- Send a reaction from the call controls: it floats up over the video on both sides
- Tap the same reaction fast for a combo, and every fifth ❤️, 🎉 or 🔥 fills the screen with hearts, confetti or flames
- Android: the phone gives a short buzz when the other person reacts
- Pick emoji from a grid in the chat

## 0.3.0 — 2026-11-10

Live subtitles while you speak, translated on the device, and calling friends.

### New

- Live subtitles: your words show to the other person while you're still speaking, recognized on the device
- Subtitles are translated on the device into the other person's language
- The last few spoken lines show above the call controls and fade after a pause; turn them off in Settings
- Sign in or create an account, add friends and see who's online
- Call a friend from the start screen: their app rings, and answering opens a private room only you two can join
- Missed calls show on the start screen, and you can rejoin a private room while the other person is still in it

### Fixed

- Android: the mic shows as available again, so mute, speech to chat and the level meter work

## 0.2.2 — 2026-11-09

Clearer voice, and switching devices during a call.

### New

- Noise suppression removes fans, keyboard and other background noise from your voice; switch it off in Settings (off by default on Android)
- Pick the mic, camera and speaker during a call, and mute the speaker
- Android: the call plays through a wired or USB headset when one is plugged in
- Settings open as a sheet, with the connection stats in it
- The ⋯ menu opens as a bottom sheet on narrow windows

### Changed

- Voice holds up better on a lossy connection: each audio packet also carries the one before it
- The room link shows only once you're in the room

### Fixed

- Screenshots on Android are saved to the gallery
- Messages mixing Chinese and English are translated
- The copy button stays visible when the room link is long
- The chosen mic and camera are the ones actually used

## 0.2.1 — 2026-11-08

Chinese text on more Android phones.

### Fixed

- Chinese text showed as boxes on some Android phones

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
