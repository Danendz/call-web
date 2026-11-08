//! Our camera: captured on a platform thread, sent into WebRTC's video source and shown in our own preview. macOS
//! uses AVFoundation directly (frames go to WebRTC without a copy), Windows uses Media Foundation through `nokhwa`,
//! Android uses Camera2 through the app's Java side.
//! `CALL_FAKE_CAMERA=1` replaces the cameras with a moving test pattern, for testing video where no camera can be
//! used (a locked screen, CI, a terminal without camera permission).

#[cfg(target_os = "android")]
pub mod android;
mod convert;
mod fake;
pub(crate) mod local;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use libwebrtc::video_frame::{VideoBuffer, VideoFrame, VideoRotation};
use libwebrtc::video_source::native::NativeVideoSource;

use crate::media::Device;
use crate::video::{Feed, Frame, VideoSink};

pub(crate) use fake::pattern as test_pattern;

/// 720p like the web's camera request, or 720x1280 when portrait is picked; at 60 fps where the camera has it (the
/// web asks for 30)
pub(crate) const WIDTH: u32 = 1280;
pub(crate) const HEIGHT: u32 = 720;
#[cfg_attr(target_os = "android", expect(dead_code, reason = "phones run their cameras at 30"))]
pub(crate) const MAX_FPS: u32 = 60;

/// The running camera's frame rate, which sets how much bitrate it gets
static FPS: AtomicU32 = AtomicU32::new(30);

pub(crate) fn fps() -> u32 {
    FPS.load(Ordering::Relaxed)
}

fn set_fps(fps: u32) {
    tracing::info!("camera at {fps} fps");
    FPS.store(fps, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraAccess {
    /// Not asked yet, or the OS prompt is up
    Pending,
    Granted,
    Denied,
    Missing,
}

/// Where captured frames go. Shared with the capture thread, which outlives nothing: dropping the [`Capture`] stops
/// it before the next frame.
pub(crate) struct Output {
    source: NativeVideoSource,
    sink: Arc<dyn VideoSink>,
    /// Sends a 9:16 crop of the camera instead of the whole 16:9 picture
    portrait: AtomicBool,
    /// Where the source's timestamps count from; WebRTC aligns them to its own clock
    epoch: Instant,
}

impl Output {
    pub(crate) fn new(source: NativeVideoSource, sink: Arc<dyn VideoSink>, portrait: bool) -> Arc<Self> {
        Arc::new(Self { source, sink, portrait: AtomicBool::new(portrait), epoch: Instant::now() })
    }

    pub(crate) fn set_portrait(&self, portrait: bool) {
        self.portrait.store(portrait, Ordering::Relaxed);
    }

    pub(crate) fn portrait(&self) -> bool {
        self.portrait.load(Ordering::Relaxed)
    }

    fn deliver<T: AsRef<dyn VideoBuffer>>(&self, buffer: T) {
        let frame = VideoFrame {
            rotation: VideoRotation::VideoRotation0,
            timestamp_us: i64::try_from(self.epoch.elapsed().as_micros()).unwrap_or(i64::MAX),
            frame_metadata: None,
            buffer,
        };
        self.source.capture_frame(&frame);
        self.sink.frame(Feed::Camera, &Frame::new(frame.buffer.as_ref(), frame.rotation));
    }
}

/// A running camera. Dropping it stops capture and turns the camera's light off.
pub(crate) struct Capture {
    _inner: Box<dyn Send>,
}

fn fake() -> bool {
    static FAKE: LazyLock<bool> = LazyLock::new(|| std::env::var_os("CALL_FAKE_CAMERA").is_some());
    *FAKE
}

/// Asks the OS for camera access if it hasn't been asked; `done` gets the answer, possibly on another thread.
pub(crate) fn request_access(done: impl FnOnce(CameraAccess) + Send + 'static) {
    if fake() {
        return done(CameraAccess::Granted);
    }
    #[cfg(target_os = "macos")]
    return mac::request_access(done);
    #[cfg(target_os = "android")]
    return android::request_access(done);
    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    done(CameraAccess::Granted)
}

pub(crate) fn list() -> Vec<Device> {
    if fake() {
        return vec![fake::device()];
    }
    #[cfg(target_os = "macos")]
    return mac::list();
    #[cfg(target_os = "windows")]
    return windows::list();
    #[cfg(target_os = "android")]
    return android::list();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "android")))]
    Vec::new()
}

/// Starts the camera with this id, or the first one if it's gone.
pub(crate) fn start(id: Option<&str>, output: Arc<Output>) -> Result<Capture, String> {
    // Only one camera may run at a time: a second open of the same device fails on Windows
    static STARTING: Mutex<()> = Mutex::new(());
    let _one = STARTING.lock().map_err(|_| "camera lock poisoned")?;
    if fake() {
        set_fps(15);
        return Ok(Capture { _inner: Box::new(fake::start(output)) });
    }
    #[cfg(target_os = "macos")]
    return mac::start(id, output).map(|c| Capture { _inner: Box::new(c) });
    #[cfg(target_os = "windows")]
    return windows::start(id, output).map(|c| Capture { _inner: Box::new(c) });
    #[cfg(target_os = "android")]
    return android::start(id, output).map(|c| Capture { _inner: Box::new(c) });
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "android")))]
    {
        let _ = (id, output);
        Err("no camera support on this platform".into())
    }
}
