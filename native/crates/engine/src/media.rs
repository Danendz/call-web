//! The WebRTC factory and the devices behind it. Audio goes through WebRTC's platform audio device: it captures the
//! picked mic, plays the other person through the default speaker, and runs WebRTC's echo canceller, noise
//! suppression and gain control on the mic, like a browser does with the web app's constraints.

use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Stable across replugging, so it's what gets remembered
    pub id: String,
    pub label: String,
}

pub struct Media {
    factory: PeerConnectionFactory,
    platform_audio: bool,
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
        Self { factory, platform_audio }
    }

    pub fn factory(&self) -> &PeerConnectionFactory {
        &self.factory
    }

    pub fn mics(&self) -> Vec<Device> {
        if !self.platform_audio {
            return vec![];
        }
        (0..u16::try_from(self.factory.recording_devices()).unwrap_or(0))
            .map(|i| Device { id: self.factory.recording_device_guid(i), label: self.factory.recording_device_name(i) })
            .filter(|d| !d.id.is_empty())
            .collect()
    }

    /// Takes effect when recording next starts; false if the mic is gone.
    pub fn select_mic(&self, id: &str) -> bool {
        self.platform_audio && self.factory.set_recording_device_by_guid(id)
    }

    /// Captures from the selected mic once a peer connection sends it.
    pub fn mic_track(&self) -> Option<RtcAudioTrack> {
        self.platform_audio.then(|| self.factory.create_device_audio_track("mic"))
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if self.platform_audio {
            self.factory.release_platform_adm();
        }
    }
}
