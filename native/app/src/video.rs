//! The engine's video frames into the window. Each feed keeps only its newest converted frame, and at most one hop
//! to the UI thread is pending per feed, so a busy UI skips frames instead of falling behind. Buffers are reused:
//! a 720p frame is 3.7 MB, and a fresh one per frame cost an allocation and a memset on top of the conversion.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use engine::{Feed, Frame, VideoSink};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ui::AppWindow;

type Buffer = SharedPixelBuffer<Rgba8Pixel>;

/// Buffers waiting to be written again. With one being written, one waiting for the UI and one on screen, two
/// spares are enough; more would only hold memory.
const SPARES: usize = 2;

struct Slot {
    latest: Mutex<Option<(Buffer, u16)>>,
    /// The frame on screen, kept so it can be written again once the next one replaces it
    shown: Mutex<Option<Buffer>>,
    spare: Mutex<Vec<Buffer>>,
    scheduled: AtomicBool,
}

impl Slot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            latest: Mutex::new(None),
            shown: Mutex::new(None),
            spare: Mutex::new(Vec::new()),
            scheduled: AtomicBool::new(false),
        })
    }

    /// A spare of this size, or a new buffer. Spares of another size (the camera switched) are dropped.
    fn take(&self, width: u32, height: u32) -> Buffer {
        let Ok(mut spare) = self.spare.lock() else { return Buffer::new(width, height) };
        spare.retain(|b| b.width() == width && b.height() == height);
        spare.pop().unwrap_or_else(|| Buffer::new(width, height))
    }

    fn give_back(&self, buffer: Buffer) {
        if let Ok(mut spare) = self.spare.lock()
            && spare.len() < SPARES
        {
            spare.push(buffer);
        }
    }
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

/// See [`COPIED`]: the share in percent.
pub fn copied_percent() -> f32 {
    COPIED.load(Ordering::Relaxed) as f32 / 100.0
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
                Some((buffer, rotation)) => (Image::from_rgba8(buffer.clone()), i32::from(rotation), Some(buffer)),
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
}

impl VideoSink for SlintSink {
    fn frame(&self, feed: Feed, frame: &Frame<'_>) {
        let slot = self.slot(feed);
        let mut buffer = slot.take(frame.width(), frame.height());
        let before = buffer.as_bytes().as_ptr();
        // Copies first if the UI still holds this buffer, so a frame being drawn is never written over
        let bytes = buffer.make_mut_bytes();
        record_copy(bytes.as_ptr() != before);
        frame.write_rgba(bytes);
        let skipped = slot.latest.lock().ok().and_then(|mut l| l.replace((buffer, frame.rotation())));
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
