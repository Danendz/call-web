//! Sharing your screen, sent alongside the camera like the web's. macOS shows its own picker (ScreenCaptureKit);
//! Windows has none, so the app lists screens and windows itself. Frames are captured at the web's 15 fps and sent
//! through a screencast video source, which tells the encoder to keep text sharp rather than motion smooth.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libwebrtc::desktop_capturer::{
    CaptureError, CaptureSource, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame,
};
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::native::yuv_helper;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::prelude::I420Buffer;
use libwebrtc::video_frame::{VideoFrame, VideoRotation};
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_track::RtcVideoTrack;

use crate::media::Device;

const FRAME_EVERY: Duration = Duration::from_millis(66);

/// Whether the OS offers its own picker, so the app doesn't list sources.
pub(crate) const SYSTEM_PICKER: bool = cfg!(target_os = "macos");

/// Screens and windows to pick from where there's no system picker; ids say which kind they are.
pub(crate) fn sources() -> Vec<Device> {
    let list = |kind: DesktopCaptureSourceType, prefix: &str| -> Vec<Device> {
        let Some(capturer) = DesktopCapturer::new(DesktopCapturerOptions::new(kind)) else { return Vec::new() };
        capturer
            .get_source_list()
            .into_iter()
            .filter(|s| kind == DesktopCaptureSourceType::Screen || !s.title().trim().is_empty())
            .map(|s| Device { id: format!("{prefix}:{}", s.id()), label: s.title() })
            .collect()
    };
    let mut all = list(DesktopCaptureSourceType::Screen, "screen");
    // Screens often come without names
    for (i, screen) in all.iter_mut().enumerate() {
        if screen.label.trim().is_empty() {
            screen.label = format!("Screen {}", i + 1);
        }
    }
    all.extend(list(DesktopCaptureSourceType::Window, "window"));
    all
}

/// A running share. Dropping it stops capture.
pub(crate) struct Share {
    stop: Arc<AtomicBool>,
}

impl Drop for Share {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts sharing `source` (an id from [`sources`]), or what the user picks in the system picker. `ended` runs once
/// if the share stops by itself: the picker was cancelled, the window closed, or the OS's "stop sharing" pressed.
pub(crate) fn start(source: Option<String>, out: NativeVideoSource, ended: impl FnOnce() + Send + 'static) -> Share {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    // `CALL_FAKE_SCREEN=1` shares a test pattern, for testing sharing where nothing can be captured (a locked
    // screen, CI, no Screen Recording permission)
    if std::env::var_os("CALL_FAKE_SCREEN").is_some() {
        let spawned = thread::Builder::new().name("fake-screen".into()).spawn(move || {
            let epoch = Instant::now();
            let mut tick = 0u32;
            while !stopped.load(Ordering::Relaxed) {
                let frame = VideoFrame {
                    rotation: VideoRotation::VideoRotation0,
                    timestamp_us: i64::try_from(epoch.elapsed().as_micros()).unwrap_or(i64::MAX),
                    frame_metadata: None,
                    buffer: crate::camera::test_pattern(1280, 800, tick.wrapping_mul(3)),
                };
                out.capture_frame(&frame);
                tick = tick.wrapping_add(1);
                thread::sleep(FRAME_EVERY);
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the fake screen: {e}");
        }
        return Share { stop };
    }
    let spawned = thread::Builder::new().name("screen".into()).spawn(move || {
        let failed = Arc::new(AtomicBool::new(false));
        if let Some(mut capturer) = open(source.as_deref(), out, failed.clone()) {
            let mut next = Instant::now();
            while !stopped.load(Ordering::Relaxed) && !failed.load(Ordering::Relaxed) {
                capturer.capture_frame();
                next += FRAME_EVERY;
                thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        }
        if !stopped.load(Ordering::Relaxed) {
            ended();
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("couldn't start screen sharing: {e}");
    }
    Share { stop }
}

fn open(source: Option<&str>, out: NativeVideoSource, failed: Arc<AtomicBool>) -> Option<DesktopCapturer> {
    let (kind, id) = match source.and_then(|s| s.split_once(':')) {
        Some(("window", id)) => (DesktopCaptureSourceType::Window, id.parse::<u64>().ok()),
        Some((_, id)) => (DesktopCaptureSourceType::Screen, id.parse::<u64>().ok()),
        None => (picker_kind(), None),
    };
    let mut options = DesktopCapturerOptions::new(kind);
    options.set_include_cursor(true);
    let mut capturer = DesktopCapturer::new(options)?;
    let chosen: Option<CaptureSource> = id.and_then(|id| capturer.get_source_list().into_iter().find(|s| s.id() == id));
    let epoch = Instant::now();
    capturer.start_capture(chosen, move |result| match result {
        Ok(frame) => {
            if let Some(i420) = to_i420(&frame) {
                let timestamp_us = i64::try_from(epoch.elapsed().as_micros()).unwrap_or(i64::MAX);
                out.capture_frame(&VideoFrame {
                    rotation: VideoRotation::VideoRotation0,
                    timestamp_us,
                    frame_metadata: None,
                    buffer: i420,
                });
            }
        }
        // A temporary error is a frame that didn't change; a permanent one ends the share
        Err(CaptureError::Temporary) => {}
        Err(CaptureError::Permanent) => failed.store(true, Ordering::Relaxed),
    });
    Some(capturer)
}

#[cfg(target_os = "macos")]
fn picker_kind() -> DesktopCaptureSourceType {
    // The system picker offers screens and windows alike
    DesktopCaptureSourceType::Generic
}

#[cfg(not(target_os = "macos"))]
fn picker_kind() -> DesktopCaptureSourceType {
    DesktopCaptureSourceType::Screen
}

/// Screen pixels (BGRA, libyuv's "ARGB") to I420, at an even size.
fn to_i420(frame: &DesktopFrame) -> Option<I420Buffer> {
    let (width, height) = (frame.width() & !1, frame.height() & !1);
    let (w, h) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let data = frame.data();
    if w == 0 || h == 0 || data.len() < frame.stride() as usize * (h as usize - 1) + w as usize * 4 {
        return None;
    }
    let mut buf = I420Buffer::new(w, h);
    let (sy, su, sv) = buf.strides();
    let (y, u, v) = buf.data_mut();
    yuv_helper::argb_to_i420(data, frame.stride(), y, sy, u, su, v, sv, width, height);
    Some(buf)
}

/// Our screen across calls: one screencast source and track for the app's life, and the share that feeds it.
pub(crate) struct LocalScreen {
    source: NativeVideoSource,
    track: RtcVideoTrack,
    share: Option<Share>,
    /// The stream it's sent in, announced in `media` messages
    pub(crate) stream_id: String,
}

impl LocalScreen {
    pub(crate) fn new(factory: &PeerConnectionFactory) -> Self {
        // Screencast: the encoder keeps detail and drops frames rather than blurring text
        let source = NativeVideoSource::new(VideoResolution { width: 1920, height: 1080 }, true);
        let track = factory.create_video_track(crate::peer::SCREEN_TRACK, source.clone());
        Self { source, track, share: None, stream_id: crate::room::random_id() }
    }

    pub(crate) fn track(&self) -> MediaStreamTrack {
        MediaStreamTrack::Video(self.track.clone())
    }

    pub(crate) fn sharing(&self) -> bool {
        self.share.is_some()
    }

    pub(crate) fn start(&mut self, source: Option<String>, ended: impl FnOnce() + Send + 'static) {
        self.share = Some(start(source, self.source.clone(), ended));
    }

    pub(crate) fn stop(&mut self) {
        self.share = None;
    }
}
