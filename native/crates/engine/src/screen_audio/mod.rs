//! The sound of a shared screen, sent in the screen's stream like the web's tab or system audio: everything the
//! computer plays except this app, which plays the other person, who would hear themselves. macOS captures it with
//! ScreenCaptureKit (macOS 13+), Windows with process loopback (Windows 10 2004+); elsewhere, or when the OS refuses,
//! the share goes out silent.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod windows;

use std::borrow::Cow;

use futures_util::FutureExt;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_source::native::NativeAudioSource;

pub(crate) const SAMPLE_RATE: u32 = 48_000;
pub(crate) const CHANNELS: u32 = 2;
/// WebRTC takes audio in 10 ms frames
#[cfg_attr(target_os = "android", expect(dead_code, reason = "the Android screen sound comes in A4"))]
const FRAME_SAMPLES: usize = (SAMPLE_RATE / 100 * CHANNELS) as usize;

/// A source for the screen's sound: no echo cancelling or noise suppression, which are for voices and would eat music
/// and game sound. Fed without a queue, so a frame goes out as soon as it's captured.
pub(crate) fn source() -> NativeAudioSource {
    let options = AudioSourceOptions { echo_cancellation: false, noise_suppression: false, auto_gain_control: false };
    NativeAudioSource::new(options, SAMPLE_RATE, CHANNELS, 0)
}

/// Cuts captured audio into the 10 ms frames WebRTC takes.
#[cfg_attr(target_os = "android", expect(dead_code, reason = "the Android screen sound comes in A4"))]
pub(crate) struct Chunker {
    source: NativeAudioSource,
    pending: Vec<i16>,
}

#[cfg_attr(target_os = "android", expect(dead_code, reason = "the Android screen sound comes in A4"))]
impl Chunker {
    pub(crate) fn new(source: NativeAudioSource) -> Self {
        Self { source, pending: Vec::with_capacity(FRAME_SAMPLES) }
    }

    /// Interleaved stereo samples, -1.0 to 1.0.
    pub(crate) fn push(&mut self, samples: impl IntoIterator<Item = f32>) {
        for sample in samples {
            self.pending.push((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
            if self.pending.len() == FRAME_SAMPLES {
                let frame = AudioFrame {
                    data: Cow::Borrowed(&self.pending),
                    sample_rate: SAMPLE_RATE,
                    num_channels: CHANNELS,
                    samples_per_channel: SAMPLE_RATE / 100,
                };
                // Without a queue the capture finishes before returning, so it never waits
                if let Some(Err(e)) = self.source.capture_frame(&frame).now_or_never() {
                    tracing::debug!("screen audio frame dropped: {}", e.message);
                }
                self.pending.clear();
            }
        }
    }
}

/// Running capture of the screen's sound. Dropping it stops capture.
pub(crate) struct Capture {
    _inner: Box<dyn Send>,
}

/// Starts capturing into `source`; None where it can't be captured, and the share goes out silent.
pub(crate) fn start(source: &NativeAudioSource) -> Option<Capture> {
    #[cfg(target_os = "macos")]
    return mac::start(Chunker::new(source.clone())).map(|c| Capture { _inner: Box::new(c) });
    #[cfg(target_os = "windows")]
    return windows::start(Chunker::new(source.clone())).map(|c| Capture { _inner: Box::new(c) });
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = source;
        None
    }
}
