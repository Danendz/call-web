//! The engine's video frames into the window. Each feed keeps only its newest converted frame, and at most one hop
//! to the UI thread is pending per feed, so a busy UI skips frames instead of falling behind.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use engine::{Feed, Frame, VideoSink};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ui::AppWindow;

struct Slot {
    latest: Mutex<Option<(SharedPixelBuffer<Rgba8Pixel>, u16)>>,
    scheduled: AtomicBool,
}

impl Slot {
    fn new() -> Arc<Self> {
        Arc::new(Self { latest: Mutex::new(None), scheduled: AtomicBool::new(false) })
    }
}

pub struct SlintSink {
    window: slint::Weak<AppWindow>,
    camera: Arc<Slot>,
    peer: Arc<Slot>,
}

impl SlintSink {
    pub fn new(window: slint::Weak<AppWindow>) -> Arc<Self> {
        Arc::new(Self { window, camera: Slot::new(), peer: Slot::new() })
    }

    fn slot(&self, feed: Feed) -> &Arc<Slot> {
        match feed {
            Feed::Camera => &self.camera,
            Feed::Peer => &self.peer,
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
            let (image, rotation) = match latest {
                Some((buffer, rotation)) => (Image::from_rgba8(buffer), i32::from(rotation)),
                None => (Image::default(), 0),
            };
            match feed {
                Feed::Camera => ui.set_self_video(image),
                Feed::Peer => {
                    ui.set_peer_video(image);
                    ui.set_peer_frame_rotation(rotation);
                }
            }
        });
    }
}

impl VideoSink for SlintSink {
    fn frame(&self, feed: Feed, frame: &Frame<'_>) {
        // A fresh buffer per frame: the one the UI shows may still be drawn from
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(frame.width(), frame.height());
        frame.write_rgba(buffer.make_mut_bytes());
        if feed == Feed::Camera {
            // Your own picture is shown like a mirror, as on the web
            let width = frame.width() as usize;
            for row in buffer.make_mut_slice().chunks_exact_mut(width.max(1)) {
                row.reverse();
            }
        }
        if let Ok(mut latest) = self.slot(feed).latest.lock() {
            *latest = Some((buffer, frame.rotation()));
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
