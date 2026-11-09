//! The WebRTC factory and the devices behind it. Audio goes through WebRTC's platform audio device: it captures the
//! picked mic, plays the other person through the picked speaker, and runs WebRTC's echo canceller, noise
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
