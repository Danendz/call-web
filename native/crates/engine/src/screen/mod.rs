//! Sharing your screen, sent alongside the camera like the web's. macOS shows its own picker (ScreenCaptureKit);
//! Windows has none, so the app lists screens and windows itself (captured with Windows Graphics Capture). Frames are
//! captured at up to 60 fps, scaled to fit 1080p, and sent through a screencast video source, which tunes the encoder
//! for text and sharp edges; what it gives up when bandwidth runs short is set in `peer.rs`. The sound goes with it
//! ([`crate::screen_audio`]). The capture itself is in `desktop.rs`; Android can't share yet.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::video_frame::{VideoFrame, VideoRotation};
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_track::RtcVideoTrack;

use crate::screen_audio;

#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
pub(crate) use desktop::sources;

/// Nothing to list on Android until MediaProjection (A4); there the share ends as soon as it starts.
#[cfg(target_os = "android")]
pub(crate) fn sources() -> Vec<crate::media::Device> {
    Vec::new()
}

/// 60 fps. A screen that doesn't change sends nearly nothing, and a game gets its motion
const FRAME_EVERY: Duration = Duration::from_micros(16_667);

/// Whether the OS offers its own picker, so the app doesn't list sources.
pub(crate) const SYSTEM_PICKER: bool = cfg!(target_os = "macos");

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
    #[cfg(not(target_os = "android"))]
    desktop::capture(source, out, stopped, ended);
    #[cfg(target_os = "android")]
    {
        let _ = (source, out, stopped);
        ended();
    }
    Share { stop, _audio: audio }
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
        // Screencast: the encoder is tuned for text and sharp edges
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
