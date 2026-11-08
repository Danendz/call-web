//! What runs depending on the screen: the mic meter on setup, the camera while a screen shows or a call sends it,
//! and speech to chat while someone's there to read it.

use super::{Controller, Internal, Phase};
use crate::level::Meter;

impl Controller {
    /// Starts and stops what depends on the screen: the mic meter on setup, the camera while a screen shows it or a
    /// call sends it. Runs after everything, so no path forgets it.
    pub(super) fn sync_devices(&mut self) {
        if self.state.phase != Phase::Setup || self.state.mics.is_empty() {
            self.meter = None;
        } else if self.meter.is_none() {
            let id = self.state.mic_id.as_ref();
            let name = self.state.mics.iter().find(|d| Some(&d.id) == id).map(|d| d.label.clone());
            self.meter = Some(Meter::start(name, self.level.clone()));
        }

        if self.camera_wanted() && !self.camera.asked {
            // Asked once per launch, on the first screen that shows the camera, like the browser's prompt
            self.camera.asked = true;
            let tx = self.internal_tx.clone();
            crate::camera::request_access(move |access| {
                let _ = tx.send(Internal::CameraAccess(access));
            });
        }
        let was_sending = self.camera.sending();
        self.camera.sync(self.camera_wanted());
        if self.camera.sending() != was_sending {
            self.send_media();
        }
        self.state.cam_on = self.camera.on;
        self.state.cam_access = self.camera.access;
        self.state.cams.clone_from(&self.camera.devices);
        self.state.cam_id = self.camera.id.clone().or_else(|| self.camera.devices.first().map(|d| d.id.clone()));
        self.state.portrait = self.camera.portrait;

        // Only while someone is there to read it, and never while muted, like the web
        self.state.remote_video = self.remote.has_camera();
        self.state.remote_screen = self.remote.has_screen();
        self.state.sharing = self.screen.sharing();

        self.sync_speech();
    }

    pub(super) fn camera_wanted(&self) -> bool {
        matches!(self.state.phase, Phase::Setup | Phase::Starting | Phase::Waiting | Phase::InCall)
    }
}
