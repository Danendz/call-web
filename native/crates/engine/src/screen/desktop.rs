//! Desktop screen capture through WebRTC's capturers: ScreenCaptureKit (with the system picker) on macOS, Windows
//! Graphics Capture on Windows. Android has none of them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use libwebrtc::desktop_capturer::{
    CaptureError, CaptureSource, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame,
};
use libwebrtc::native::yuv_helper;
use libwebrtc::prelude::I420Buffer;
use libwebrtc::video_frame::{VideoFrame, VideoRotation};
use libwebrtc::video_source::native::NativeVideoSource;

use super::FRAME_EVERY;
use crate::media::Device;

/// Bigger screens are scaled to fit, which is about their size in points on a Retina Mac: text stays readable, and
/// the encoder isn't handed four times the pixels at 60 fps
const MAX_WIDTH: u32 = 1920;
const MAX_HEIGHT: u32 = 1080;

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

/// Captures `source` on its own thread until `stopped`; `ended` runs if capture stops by itself.
pub(super) fn capture(
    source: Option<String>,
    out: NativeVideoSource,
    stopped: Arc<AtomicBool>,
    ended: impl FnOnce() + Send + 'static,
) {
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
