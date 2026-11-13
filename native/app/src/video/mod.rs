//! The engine's video frames into the window. Each feed keeps only its newest converted frame, and at most one hop
//! to the UI thread is pending per feed, so a busy UI skips frames instead of falling behind. Buffers are reused:
//! a 720p frame is 3.7 MB, and a fresh one per frame cost an allocation and a memset on top of the conversion.
//! Frames nobody would see (the window is hidden, or the layout doesn't show that feed) aren't converted at all.
//! When the window draws through wgpu, frames are converted on the GPU instead ([`gpu`]), with the same slots.

#[cfg(not(target_os = "android"))]
pub mod gpu;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use engine::{Feed, Frame, VideoSink};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ui::AppWindow;

type Buffer = SharedPixelBuffer<Rgba8Pixel>;

/// A converted frame: pixels in memory, or a texture the GPU drew.
enum Picture {
    Pixels(Buffer),
    #[cfg(not(target_os = "android"))]
    Texture(gpu::Target),
}

impl Picture {
    fn image(&self) -> Image {
        match self {
            Self::Pixels(buffer) => Image::from_rgba8(buffer.clone()),
            #[cfg(not(target_os = "android"))]
            Self::Texture(target) => target.image(),
        }
    }
}

/// Frames waiting to be written again. With one being written, one waiting for the UI and one on screen, two
/// spares are enough; more would only hold memory.
const SPARES: usize = 2;

struct Slot {
    latest: Mutex<Option<(Picture, u16)>>,
    /// The frame on screen, kept so it can be written again once the next one replaces it
    shown: Mutex<Option<Picture>>,
    spare: Mutex<Vec<Picture>>,
    scheduled: AtomicBool,
    /// Whether the screen on show draws this feed; see [`SlintSink::set_seen`]
    seen: AtomicBool,
}

impl Slot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            latest: Mutex::new(None),
            shown: Mutex::new(None),
            spare: Mutex::new(Vec::new()),
            scheduled: AtomicBool::new(false),
            seen: AtomicBool::new(true),
        })
    }

    /// A spare that `fits`. Spares that don't (the camera switched size, or the renderer changed) are dropped.
    fn take(&self, fits: impl Fn(&Picture) -> bool) -> Option<Picture> {
        let mut spare = self.spare.lock().ok()?;
        spare.retain(fits);
        spare.pop()
    }

    fn take_buffer(&self, width: u32, height: u32) -> Buffer {
        match self.take(|p| matches!(p, Picture::Pixels(b) if b.width() == width && b.height() == height)) {
            Some(Picture::Pixels(buffer)) => buffer,
            _ => Buffer::new(width, height),
        }
    }

    fn give_back(&self, picture: Picture) {
        if let Ok(mut spare) = self.spare.lock()
            && spare.len() < SPARES
        {
            spare.push(picture);
        }
    }
}

/// Whether any of the window is on screen: false while it's minimized or covered, or on Android stopped
static WINDOW_SHOWN: AtomicBool = AtomicBool::new(true);

pub fn set_window_shown(shown: bool) {
    WINDOW_SHOWN.store(shown, Ordering::Relaxed);
}

/// The share of frames whose reused buffer the UI still held, so Slint copied it before it could be written, in
/// hundredths of a percent, smoothed. Near zero when the pool works; shown in the stats overlay.
static COPIED: AtomicU32 = AtomicU32::new(0);

fn record_copy(copied: bool) {
    let sample = if copied { 10_000 } else { 0 };
    // Two feeds racing may lose a sample, which doesn't matter here
    let old = COPIED.load(Ordering::Relaxed);
    COPIED.store((old * 15 + sample) / 16, Ordering::Relaxed);
}

/// See [`COPIED`]: the share in percent, or `None` while the GPU converts and nothing is copied.
pub fn copied_percent() -> Option<f32> {
    #[cfg(not(target_os = "android"))]
    if gpu::get().is_some() {
        return None;
    }
    Some(COPIED.load(Ordering::Relaxed) as f32 / 100.0)
}

pub struct SlintSink {
    window: slint::Weak<AppWindow>,
    camera: Arc<Slot>,
    peer: Arc<Slot>,
    screen: Arc<Slot>,
}

impl SlintSink {
    pub fn new(window: slint::Weak<AppWindow>) -> Arc<Self> {
        Arc::new(Self { window, camera: Slot::new(), peer: Slot::new(), screen: Slot::new() })
    }

    /// Which feeds the screen on show draws. One left out keeps its last frame, which shows for a moment when it's
    /// drawn again, until the next one arrives.
    pub fn set_seen(&self, camera: bool, peer: bool, screen: bool) {
        self.camera.seen.store(camera, Ordering::Relaxed);
        self.peer.seen.store(peer, Ordering::Relaxed);
        self.screen.seen.store(screen, Ordering::Relaxed);
    }

    fn slot(&self, feed: Feed) -> &Arc<Slot> {
        match feed {
            Feed::Camera => &self.camera,
            Feed::Peer => &self.peer,
            Feed::Screen => &self.screen,
        }
    }

    /// Hands the slot's newest frame to the UI, or clears the feed when there is none.
    fn schedule(&self, feed: Feed) {
        let slot = self.slot(feed).clone();
        if slot.scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            slot.scheduled.store(false, Ordering::Release);
            let latest = slot.latest.lock().ok().and_then(|mut l| l.take());
            let (image, rotation, shown) = match latest {
                Some((picture, rotation)) => (picture.image(), i32::from(rotation), Some(picture)),
                None => (Image::default(), 0, None),
            };
            match feed {
                Feed::Camera => {
                    ui.set_self_video(image);
                    ui.set_self_frame_rotation(rotation);
                }
                Feed::Peer => {
                    ui.set_peer_video(image);
                    ui.set_peer_frame_rotation(rotation);
                }
                Feed::Screen => ui.set_screen_video(image),
            }
            // The frame it replaced is no longer drawn
            let replaced = slot.shown.lock().ok().and_then(|mut s| std::mem::replace(&mut *s, shown));
            if let Some(replaced) = replaced {
                slot.give_back(replaced);
            }
        });
    }

    fn convert(&self, slot: &Slot, frame: &Frame<'_>) -> Picture {
        #[cfg(not(target_os = "android"))]
        if let Some(gpu) = gpu::get() {
            return frame.with_planes(|planes| {
                let layout = gpu::Layout::of(&planes);
                let target = match slot.take(|p| matches!(p, Picture::Texture(t) if gpu.fits(t, layout))) {
                    Some(Picture::Texture(target)) => target,
                    _ => gpu.target(layout),
                };
                gpu.convert(&target, &planes);
                Picture::Texture(target)
            });
        }
        let mut buffer = slot.take_buffer(frame.width(), frame.height());
        let before = buffer.as_bytes().as_ptr();
        // Copies first if the UI still holds this buffer, so a frame being drawn is never written over
        let bytes = buffer.make_mut_bytes();
        record_copy(bytes.as_ptr() != before);
        frame.write_rgba(bytes);
        Picture::Pixels(buffer)
    }
}

/// The pixels of a frame on screen, for a screenshot.
pub fn pixels(image: &Image) -> Option<Buffer> {
    #[cfg(not(target_os = "android"))]
    if let Some(pixels) = gpu::read(image) {
        return Some(pixels);
    }
    image.to_rgba8()
}

impl VideoSink for SlintSink {
    fn frame(&self, feed: Feed, frame: &Frame<'_>) {
        let slot = self.slot(feed);
        if !slot.seen.load(Ordering::Relaxed) || !WINDOW_SHOWN.load(Ordering::Relaxed) {
            return;
        }
        let picture = self.convert(slot, frame);
        let skipped = slot.latest.lock().ok().and_then(|mut l| l.replace((picture, frame.rotation())));
        // The UI never took the frame before this one
        if let Some((skipped, _)) = skipped {
            slot.give_back(skipped);
        }
        self.schedule(feed);
    }

    fn clear(&self, feed: Feed) {
        if let Ok(mut latest) = self.slot(feed).latest.lock() {
            *latest = None;
        }
        self.schedule(feed);
    }
}
