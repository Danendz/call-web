//! Video frames on their way to the screen. The engine hands each feed's newest frame to a [`VideoSink`], which
//! converts it straight into the UI's own buffer: nothing is queued, a slow UI just skips frames.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use futures_util::StreamExt;
use libwebrtc::prelude::RtcVideoTrack;
use libwebrtc::video_frame::native::VideoFrameBufferExt;
use libwebrtc::video_frame::{VideoBuffer, VideoFormatType, VideoRotation};
use libwebrtc::video_stream::native::NativeVideoStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feed {
    /// Our camera, for the preview and our tile
    Camera,
    /// The other person's camera
    Peer,
    /// The other person's shared screen
    Screen,
}

/// One frame, borrowed for the length of a [`VideoSink::frame`] call.
pub struct Frame<'a> {
    buffer: &'a dyn VideoBuffer,
    rotation: u16,
}

impl<'a> Frame<'a> {
    pub(crate) fn new(buffer: &'a dyn VideoBuffer, rotation: VideoRotation) -> Self {
        let rotation = match rotation {
            VideoRotation::VideoRotation0 => 0,
            VideoRotation::VideoRotation90 => 90,
            VideoRotation::VideoRotation180 => 180,
            VideoRotation::VideoRotation270 => 270,
        };
        Self { buffer, rotation }
    }

    pub fn width(&self) -> u32 {
        self.buffer.width()
    }

    pub fn height(&self) -> u32 {
        self.buffer.height()
    }

    /// Clockwise degrees the picture must be turned to stand upright: phones send it sideways with this note.
    pub fn rotation(&self) -> u16 {
        self.rotation
    }

    /// Converts into `dst`: `width * height` RGBA pixels, rows packed. Does nothing if `dst` is the wrong size.
    pub fn write_rgba(&self, dst: &mut [u8]) {
        let (width, height) = (self.width(), self.height());
        if dst.len() != width as usize * height as usize * 4 {
            return;
        }
        let started = Instant::now();
        // libyuv names formats by the 32-bit word, so its "ABGR" is R, G, B, A in memory: Slint's RGBA
        let format = VideoFormatType::ABGR;
        let (stride, w, h) = (width * 4, width as i32, height as i32);
        let b = self.buffer;
        if let Some(buf) = b.as_i420() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_nv12() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_native() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_i420a() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_i422() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_i444() {
            buf.to_argb(format, dst, stride, w, h);
        } else if let Some(buf) = b.as_i010() {
            buf.to_argb(format, dst, stride, w, h);
        }
        CONVERT_US.record(started.elapsed().as_micros());
    }

    /// Hands `f` the frame as I420, for a shader to turn into RGBA. A VP8 frame is I420 already and is only borrowed;
    /// any other is copied into one first (a decoded H.264 frame on macOS is NV12), which still costs far less than
    /// converting to RGBA. Timed like [`Self::write_rgba`], `f` included. None for a format there's no copy for.
    pub fn with_i420<R>(&self, f: impl FnOnce(&I420<'_>) -> R) -> Option<R> {
        let started = Instant::now();
        let b = self.buffer;
        let copied;
        let buf = match b.as_i420() {
            Some(buf) => buf,
            None => {
                copied = if let Some(buf) = b.as_nv12() {
                    buf.to_i420()
                } else if let Some(buf) = b.as_native() {
                    buf.to_i420()
                } else if let Some(buf) = b.as_i420a() {
                    buf.to_i420()
                } else if let Some(buf) = b.as_i422() {
                    buf.to_i420()
                } else if let Some(buf) = b.as_i444() {
                    buf.to_i420()
                } else {
                    b.as_i010()?.to_i420()
                };
                &copied
            }
        };
        let (y, u, v) = buf.data();
        let result = f(&I420 { width: self.width(), height: self.height(), y, u, v, strides: buf.strides() });
        CONVERT_US.record(started.elapsed().as_micros());
        Some(result)
    }
}

/// A frame's three planes: Y at full size, U and V at half the width and height, rounded up.
pub struct I420<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    /// Bytes from one row to the next in Y, U and V
    pub strides: (u32, u32, u32),
}

/// Where frames go to be shown. Called on video threads, so it must be quick and must not queue: keep the newest
/// frame, drop the rest.
pub trait VideoSink: Send + Sync + 'static {
    fn frame(&self, feed: Feed, frame: &Frame<'_>);
    /// The feed stopped: show nothing for it.
    fn clear(&self, feed: Feed);
}

/// How long converting a frame for the screen takes, smoothed, for the stats overlay.
pub(crate) static CONVERT_US: Smoothed = Smoothed(AtomicU32::new(0));
/// How long turning a camera frame into the I420 frame WebRTC encodes takes, smoothed, for the stats overlay.
pub(crate) static CAMERA_US: Smoothed = Smoothed(AtomicU32::new(0));

pub(crate) struct Smoothed(AtomicU32);

impl Smoothed {
    pub(crate) fn record(&self, sample: u128) {
        let sample = u32::try_from(sample).unwrap_or(u32::MAX);
        let old = self.0.load(Ordering::Relaxed);
        let new = if old == 0 { sample } else { (old * 7 + sample) / 8 };
        self.0.store(new, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Shows a remote video track until the returned task is aborted.
pub(crate) fn pump(track: RtcVideoTrack, feed: Feed, sink: Arc<dyn VideoSink>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // The default queue holds one frame and drops older ones, which is what the screen wants
        let mut stream = NativeVideoStream::new(track);
        while let Some(frame) = stream.next().await {
            sink.frame(feed, &Frame::new(frame.buffer.as_ref(), frame.rotation));
        }
        sink.clear(feed);
    })
}
