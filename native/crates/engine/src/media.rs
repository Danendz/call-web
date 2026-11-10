//! The WebRTC factory and the devices behind it. Audio goes through WebRTC's platform audio device: it captures the
//! picked mic, plays the other person through the picked speaker, and runs WebRTC's echo canceller, noise
//! suppression and gain control on the mic, like a browser does with the web app's constraints. With noise
//! suppression switched on, DeepFilterNet replaces WebRTC's own after the other two.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use webrtc_sys::capture_processor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Stable across replugging, so it's what gets remembered
    pub id: String,
    pub label: String,
}

pub struct Media {
    factory: PeerConnectionFactory,
    platform_audio: bool,
    denoise: Arc<Denoise>,
    /// WebRTC shuts the audio device down once the last peer connection is gone, and on Android LiveKit's proxy
    /// never starts it again: the next call crashed starting audio. A connection that never connects keeps it up.
    #[cfg(target_os = "android")]
    _keep_audio: Option<libwebrtc::peer_connection::PeerConnection>,
}

impl Media {
    pub fn new() -> Self {
        let factory = PeerConnectionFactory::default();
        let platform_audio = factory.acquire_platform_adm();
        if platform_audio {
            factory.set_adm_recording_enabled(true);
            factory.set_adm_playout_enabled(true);
        } else {
            tracing::warn!("no platform audio device: the call will be silent");
        }
        #[cfg(target_os = "android")]
        let _keep_audio = factory
            .create_peer_connection(libwebrtc::peer_connection_factory::RtcConfiguration::default())
            .inspect_err(|e| tracing::warn!("no idle connection to keep audio up: {e:?}"))
            .ok();
        Self {
            factory,
            platform_audio,
            denoise: Arc::default(),
            #[cfg(target_os = "android")]
            _keep_audio,
        }
    }

    pub fn factory(&self) -> &PeerConnectionFactory {
        &self.factory
    }

    /// None on Android, where the mic follows the call's audio route (a headset brings its own; `Shim.kt`).
    pub fn mics(&self) -> Vec<Device> {
        if !self.platform_audio || cfg!(target_os = "android") {
            return vec![];
        }
        (0..u16::try_from(self.factory.recording_devices()).unwrap_or(0))
            .map(|i| Device { id: self.factory.recording_device_guid(i), label: self.factory.recording_device_name(i) })
            .filter(|d| !d.id.is_empty())
            .collect()
    }

    /// A mic can be captured. True on Android without a list: its one mic is the audio route's.
    pub fn has_mic(&self) -> bool {
        self.platform_audio && (cfg!(target_os = "android") || !self.mics().is_empty())
    }

    /// False if the mic is gone. A call's capture restarts on it: the ADM won't change device while recording.
    pub fn select_mic(&self, id: &str) -> bool {
        if !self.platform_audio {
            return false;
        }
        let live = self.factory.recording_is_initialized();
        if live && !self.factory.stop_recording() {
            tracing::warn!("couldn't stop recording to switch mic");
        }
        let picked = self.factory.set_recording_device_by_guid(id);
        // Back on, on the new mic or the old one
        if live && !(self.factory.init_recording() && self.factory.start_recording()) {
            tracing::warn!("couldn't restart recording after switching mic");
        }
        picked
    }

    /// Where the call plays. None on Android, whose outputs are the call's audio routes instead (`Shim.kt`).
    pub fn speakers(&self) -> Vec<Device> {
        if !self.platform_audio || cfg!(target_os = "android") {
            return vec![];
        }
        (0..u16::try_from(self.factory.playout_devices()).unwrap_or(0))
            .map(|i| Device { id: self.factory.playout_device_guid(i), label: self.factory.playout_device_name(i) })
            .filter(|d| !d.id.is_empty())
            .collect()
    }

    /// False if the speaker is gone. A call's playout restarts on it, like the mic's capture.
    pub fn select_speaker(&self, id: &str) -> bool {
        if !self.platform_audio {
            return false;
        }
        let live = self.factory.playout_is_initialized();
        if live && !self.factory.stop_playout() {
            tracing::warn!("couldn't stop playout to switch speaker");
        }
        let picked = self.factory.set_playout_device_by_guid(id);
        if live && !(self.factory.init_playout() && self.factory.start_playout()) {
            tracing::warn!("couldn't restart playout after switching speaker");
        }
        picked
    }

    /// DeepFilterNet on the mic instead of WebRTC's noise suppression, or back. Live: the next 10 ms frame follows. The
    /// model loads the first time, off this thread; WebRTC's stays on until it has, and if it can't.
    pub fn set_noise_suppression(&self, on: bool) {
        let denoise = &self.denoise;
        denoise.on.store(on, Ordering::Relaxed);
        if on && denoise.model.load(Ordering::Relaxed) == MODEL_NONE {
            denoise.model.store(MODEL_LOADING, Ordering::Relaxed);
            let denoise = denoise.clone();
            std::thread::spawn(move || {
                let loaded = denoise::Denoiser::new()
                    .inspect_err(|e| tracing::warn!("noise suppression model didn't load: {e}"))
                    .ok();
                let model = if loaded.is_some() { MODEL_READY } else { MODEL_FAILED };
                if let Some(denoiser) = loaded {
                    capture_processor::set_capture_processor(Some(processor(denoiser, denoise.on.clone())));
                }
                denoise.model.store(model, Ordering::Relaxed);
                denoise.apply();
            });
        }
        denoise.apply();
    }

    /// Captures from the selected mic once a peer connection sends it.
    pub fn mic_track(&self) -> Option<RtcAudioTrack> {
        self.platform_audio.then(|| self.factory.create_device_audio_track("mic"))
    }
}

const MODEL_NONE: u8 = 0;
const MODEL_LOADING: u8 = 1;
const MODEL_READY: u8 = 2;
const MODEL_FAILED: u8 = 3;

#[derive(Default)]
struct Denoise {
    /// The setting; the processor reads it on the audio thread
    on: Arc<AtomicBool>,
    model: AtomicU8,
}

impl Denoise {
    /// WebRTC's noise suppression is off only while DeepFilterNet runs in its place.
    fn apply(&self) {
        let ours = self.on.load(Ordering::Relaxed) && self.model.load(Ordering::Relaxed) == MODEL_READY;
        capture_processor::set_builtin_noise_suppression(!ours);
    }
}

/// Runs on WebRTC's audio thread, after its echo canceller and gain control. Frames at a rate other than the model's
/// (WebRTC processes at 48 kHz when the mic gives it that, as desktop and phone mics do) pass through.
fn processor(mut denoiser: denoise::Denoiser, on: Arc<AtomicBool>) -> capture_processor::CaptureProcessor {
    let mut warned = false;
    Box::new(move |frame, rate| {
        if !on.load(Ordering::Relaxed) {
            return;
        }
        if rate != denoise::SAMPLE_RATE {
            if !warned {
                tracing::warn!("noise suppression skipped: the mic is processed at {rate} Hz");
                warned = true;
            }
            return;
        }
        // WebRTC's floats are in the 16-bit range
        denoiser.process(frame, 32768.0);
    })
}

impl Drop for Media {
    fn drop(&mut self) {
        if self.platform_audio {
            self.factory.release_platform_adm();
        }
    }
}
