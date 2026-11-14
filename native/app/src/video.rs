//! The engine's video frames into the window. Each feed keeps only its newest converted frame, and at most one hop
//! to the UI thread is pending per feed, so a busy UI skips frames instead of falling behind. Buffers are reused:
//! a 720p frame is 3.7 MB, and a fresh one per frame cost an allocation and a memset on top of the conversion.
//! Frames nobody would see (the window is hidden, or the layout doesn't show that feed) aren't converted at all.
//! When the window draws with wgpu, frames are converted by the GPU into textures instead (`gpu.rs`), pooled the
//! same way.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use engine::{Feed, Frame, VideoSink};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ui::AppWindow;

type Buffer = SharedPixelBuffer<Rgba8Pixel>;

/// A converted frame: pixels in memory, or a texture on the GPU with the device it was made on.
pub enum Pixels {
    Cpu(Buffer),
    #[cfg(not(target_os = "android"))]
    Gpu(wgpu::Texture, wgpu::Device),
}

#[cfg(not(target_os = "android"))]
use slint::wgpu_30::wgpu;
/// The device the GPU converts on; Android has none
#[cfg(not(target_os = "android"))]
type Device = wgpu::Device;
#[cfg(target_os = "android")]
type Device = std::convert::Infallible;

impl Pixels {
    /// Whether this can be written again with a frame of this size: by the CPU, or by the GPU on `device`
    fn fits(&self, width: u32, height: u32, device: Option<&Device>) -> bool {
        match self {
            Self::Cpu(b) => device.is_none() && b.width() == width && b.height() == height,
            #[cfg(not(target_os = "android"))]
            Self::Gpu(t, d) => device == Some(d) && t.width() == width && t.height() == height,
        }
    }

    /// The frame an image on screen shows, for a screenshot.
    pub fn of(image: &Image) -> Option<Self> {
        if let Some(buffer) = image.to_rgba8() {
            return Some(Self::Cpu(buffer));
        }
        #[cfg(not(target_os = "android"))]
        if let Some(texture) = image.to_wgpu_30_texture() {
            return Some(Self::Gpu(texture, crate::gpu::converter()?.device().clone()));
        }
        None
    }

    /// The frame's pixels in memory. A texture is read back from the GPU, which waits for it: not on the UI thread.
    pub fn to_buffer(&self) -> Option<Buffer> {
        match self {
            Self::Cpu(b) => Some(b.clone()),
            #[cfg(not(target_os = "android"))]
            // A texture from a renderer since torn down can't be read
            Self::Gpu(t, d) => crate::gpu::converter().filter(|c| c.device() == d)?.read(t),
        }
    }

    fn image(&self) -> Image {
        match self {
            Self::Cpu(b) => Image::from_rgba8(b.clone()),
            #[cfg(not(target_os = "android"))]
            Self::Gpu(t, _) => Image::try_from(t.clone()).unwrap_or_default(),
        }
    }
}

/// Buffers waiting to be written again. With one being written, one waiting for the UI and one on screen, two
/// spares are enough; more would only hold memory.
const SPARES: usize = 2;

struct Slot {
    latest: Mutex<Option<(Pixels, u16)>>,
    /// The frame on screen, kept so it can be written again once the next one replaces it
    shown: Mutex<Option<Pixels>>,
    spare: Mutex<Vec<Pixels>>,
    /// The plane textures the GPU converts from
    #[cfg(not(target_os = "android"))]
    planes: Mutex<Option<crate::gpu::Planes>>,
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
            #[cfg(not(target_os = "android"))]
            planes: Mutex::new(None),
            scheduled: AtomicBool::new(false),
            seen: AtomicBool::new(true),
        })
    }

    /// A spare of this size and kind. Spares that don't fit (the camera switched, or the renderer) are dropped.
    fn take(&self, width: u32, height: u32, device: Option<&Device>) -> Option<Pixels> {
        let mut spare = self.spare.lock().ok()?;
        spare.retain(|p| p.fits(width, height, device));
        spare.pop()
    }

    fn give_back(&self, buffer: Pixels) {
        if let Ok(mut spare) = self.spare.lock()
            && spare.len() < SPARES
        {
            spare.push(buffer);
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
                Some((pixels, rotation)) => (pixels.image(), i32::from(rotation), Some(pixels)),
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
        if !slot.seen.load(Ordering::Relaxed) || !WINDOW_SHOWN.load(Ordering::Relaxed) {
            return;
        }
        let (width, height) = (frame.width(), frame.height());
        #[cfg(not(target_os = "android"))]
        if let Some(gpu) = crate::gpu::converter() {
            let texture = match slot.take(width, height, Some(gpu.device())) {
                Some(Pixels::Gpu(texture, _)) => texture,
                _ => gpu.texture(width, height),
            };
            let Ok(mut planes) = slot.planes.lock() else { return };
            if frame.with_planes(|yuv| gpu.convert(&mut planes, yuv, &texture)) == Some(true) {
                self.show(feed, Pixels::Gpu(texture, gpu.device().clone()), frame.rotation());
            }
            return;
        }
        let mut buffer = match slot.take(width, height, None) {
            Some(Pixels::Cpu(buffer)) => buffer,
            _ => Buffer::new(width, height),
        };
        let before = buffer.as_bytes().as_ptr();
        // Copies first if the UI still holds this buffer, so a frame being drawn is never written over
        let bytes = buffer.make_mut_bytes();
        record_copy(bytes.as_ptr() != before);
        frame.write_rgba(bytes);
        self.show(feed, Pixels::Cpu(buffer), frame.rotation());
    }

    fn clear(&self, feed: Feed) {
        if let Ok(mut latest) = self.slot(feed).latest.lock() {
            *latest = None;
        }
        self.schedule(feed);
    }
}

impl SlintSink {
    /// Makes `pixels` the feed's newest frame and hands it to the UI.
    fn show(&self, feed: Feed, pixels: Pixels, rotation: u16) {
        let slot = self.slot(feed);
        let skipped = slot.latest.lock().ok().and_then(|mut l| l.replace((pixels, rotation)));
        // The UI never took the frame before this one
        if let Some((skipped, _)) = skipped {
            slot.give_back(skipped);
        }
        self.schedule(feed);
    }
}
