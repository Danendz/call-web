//! Sharing your screen, sent alongside the camera like the web's. macOS shows its own picker (ScreenCaptureKit);
//! Windows has none, so the app lists screens and windows itself (captured with Windows Graphics Capture). Frames are
//! captured at up to 60 fps, scaled to fit 1080p, and sent through a screencast video source, which tells the encoder
//! to keep text sharp: when bandwidth runs short it drops frames, not detail. The sound goes with it
//! ([`crate::screen_audio`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_track::RtcAudioTrack;
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
use crate::screen_audio;

/// 60 fps. A screen that doesn't change sends nearly nothing, and a game gets its motion
const FRAME_EVERY: Duration = Duration::from_micros(16_667);
/// Bigger screens are scaled to fit, which is about their size in points on a Retina Mac: text stays readable, and
/// the encoder isn't handed four times the pixels at 60 fps
const MAX_WIDTH: u32 = 1920;
const MAX_HEIGHT: u32 = 1080;

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
    _audio: Option<screen_audio::Capture>,
}

impl Drop for Share {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts sharing `source` (an id from [`sources`]), or what the user picks in the system picker. `ended` runs once
/// if the share stops by itself: the picker was cancelled, the window closed, or the OS's "stop sharing" pressed.
pub(crate) fn start(
    source: Option<String>,
    out: NativeVideoSource,
    audio: &NativeAudioSource,
    ended: impl FnOnce() + Send + 'static,
) -> Share {
    let stop = Arc::new(AtomicBool::new(false));
    let audio = screen_audio::start(audio);
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
        return Share { stop, _audio: audio };
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
    Share { stop, _audio: audio }
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

/// Screen pixels (BGRA, libyuv's "ARGB") to I420, at an even size that fits [`MAX_WIDTH`] x [`MAX_HEIGHT`].
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
    let (fit_w, fit_h) = fit(w, h);
    if (fit_w, fit_h) == (w, h) {
        return Some(buf);
    }
    Some(buf.scale(fit_w as i32, fit_h as i32))
}

/// The even size that keeps the picture's shape and fits [`MAX_WIDTH`] x [`MAX_HEIGHT`].
fn fit(width: u32, height: u32) -> (u32, u32) {
    if width <= MAX_WIDTH && height <= MAX_HEIGHT {
        return (width, height);
    }
    let scale = f64::min(f64::from(MAX_WIDTH) / f64::from(width), f64::from(MAX_HEIGHT) / f64::from(height));
    let even = |n: u32| ((f64::from(n) * scale) as u32).max(2) & !1;
    (even(width), even(height))
}

/// What a share sends, in the screen's own stream.
pub(crate) struct ScreenTracks {
    pub(crate) video: MediaStreamTrack,
    pub(crate) audio: MediaStreamTrack,
}

/// Our screen across calls: one screencast source and track for the app's life, and the share that feeds it.
pub(crate) struct LocalScreen {
    source: NativeVideoSource,
    track: RtcVideoTrack,
    audio_source: NativeAudioSource,
    audio_track: RtcAudioTrack,
    share: Option<Share>,
    /// The stream it's sent in, announced in `media` messages
    pub(crate) stream_id: String,
}

impl LocalScreen {
    pub(crate) fn new(factory: &PeerConnectionFactory) -> Self {
        // Screencast: the encoder keeps detail and drops frames rather than blurring text
        let source = NativeVideoSource::new(VideoResolution { width: 1920, height: 1080 }, true);
        let track = factory.create_video_track(crate::peer::SCREEN_TRACK, source.clone());
        let audio_source = screen_audio::source();
        let audio_track = factory.create_audio_track(crate::peer::SCREEN_AUDIO_TRACK, audio_source.clone());
        Self { source, track, audio_source, audio_track, share: None, stream_id: crate::room::random_id() }
    }

    pub(crate) fn tracks(&self) -> ScreenTracks {
        ScreenTracks {
            video: MediaStreamTrack::Video(self.track.clone()),
            audio: MediaStreamTrack::Audio(self.audio_track.clone()),
        }
    }

    pub(crate) fn sharing(&self) -> bool {
        self.share.is_some()
    }

    pub(crate) fn start(&mut self, source: Option<String>, ended: impl FnOnce() + Send + 'static) {
        self.share = Some(start(source, self.source.clone(), &self.audio_source, ended));
    }

    pub(crate) fn stop(&mut self) {
        self.share = None;
    }
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn a_retina_screen_is_scaled_to_fit_1080p_at_an_even_size() {
        assert_eq!(fit(3024, 1964), (1662, 1080));
        assert_eq!(fit(5120, 1440), (1920, 540));
        assert_eq!(fit(1920, 1080), (1920, 1080));
        assert_eq!(fit(1280, 800), (1280, 800));
    }
}
