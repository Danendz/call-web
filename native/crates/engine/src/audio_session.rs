//! iOS's audio session has to be set for recording before a mic opens. WebRTC sets it only once a call's sound
//! starts, so before that the setup screen's meter found no mic, and iOS offered no Bluetooth headset to pick.

use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionModeVoiceChat,
    AVAudioSessionSetActiveOptions,
};

/// Sets the session as WebRTC will for the call, Bluetooth headsets included, and turns it on. Leaves a session
/// already set for recording as it is, so a call's settings stay.
pub fn prepare() {
    // SAFETY: the shared session lives for the app's whole life and may be used from any thread; the category and
    // mode are valid NSString constants
    let done = unsafe {
        let session = AVAudioSession::sharedInstance();
        match (AVAudioSessionCategoryPlayAndRecord, AVAudioSessionModeVoiceChat) {
            (Some(record), Some(voice)) if *session.category() != *record => {
                session.setCategory_mode_options_error(record, voice, AVAudioSessionCategoryOptions::AllowBluetoothHFP)
            }
            _ => Ok(()),
        }
        .and_then(|()| session.setActive_error(true))
    };
    if let Err(e) = done {
        tracing::warn!("audio session not set for recording: {}", e.localizedDescription());
    }
}

/// Turns the session off once the setup and call screens are gone, so other apps' sound comes back.
pub fn release() {
    // SAFETY: as in `prepare`
    let done = unsafe {
        AVAudioSession::sharedInstance()
            .setActive_withOptions_error(false, AVAudioSessionSetActiveOptions::NotifyOthersOnDeactivation)
    };
    if let Err(e) = done {
        tracing::info!("audio session still on: {}", e.localizedDescription());
    }
}
