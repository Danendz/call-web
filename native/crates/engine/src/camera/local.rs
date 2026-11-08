//! Our camera across the screens: one video source and track for the app's life, a capture that runs while a screen
//! shows or sends it, and what the user picked (which camera, on or off, portrait).

use std::sync::Arc;

use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_track::RtcVideoTrack;

use super::{CameraAccess, Capture, HEIGHT, Output, WIDTH};
use crate::media::Device;
use crate::video::{Feed, VideoSink};

pub(crate) struct LocalCamera {
    output: Arc<Output>,
    track: RtcVideoTrack,
    sink: Arc<dyn VideoSink>,
    capture: Option<Capture>,
    pub(crate) access: CameraAccess,
    /// The OS has been asked for access this launch
    pub(crate) asked: bool,
    pub(crate) devices: Vec<Device>,
    /// The picked camera; the first one when None or gone
    pub(crate) id: Option<String>,
    pub(crate) on: bool,
    pub(crate) portrait: bool,
}

impl LocalCamera {
    pub(crate) fn new(factory: &PeerConnectionFactory, sink: Arc<dyn VideoSink>) -> Self {
        let source = NativeVideoSource::new(VideoResolution { width: WIDTH, height: HEIGHT }, false);
        let track = factory.create_video_track("camera", source.clone());
        Self {
            output: Output::new(source, sink.clone(), false),
            track,
            sink,
            capture: None,
            access: CameraAccess::Pending,
            asked: false,
            devices: Vec::new(),
            id: None,
            on: true,
            portrait: false,
        }
    }

    /// Sent in every call, even with the camera off or missing: turning it on later then needs no renegotiation.
    pub(crate) fn track(&self) -> MediaStreamTrack {
        MediaStreamTrack::Video(self.track.clone())
    }

    /// The camera is on, allowed and running: what the `media` message reports as `camOn`.
    pub(crate) fn sending(&self) -> bool {
        self.on && self.capture.is_some()
    }

    pub(crate) fn set_access(&mut self, access: CameraAccess) {
        self.access = access;
        if access == CameraAccess::Granted {
            self.devices = super::list();
            if self.devices.is_empty() {
                self.access = CameraAccess::Missing;
            }
        }
    }

    /// Runs the camera while `wanted` (a screen shows or sends it) and it's on and allowed; stops it otherwise, which
    /// also turns its light off.
    pub(crate) fn sync(&mut self, wanted: bool) {
        let run = wanted && self.on && self.access == CameraAccess::Granted;
        if run && self.capture.is_none() {
            match super::start(self.id.as_deref(), self.output.clone()) {
                Ok(capture) => self.capture = Some(capture),
                Err(e) => {
                    tracing::warn!("camera failed to start: {e}");
                    self.access = CameraAccess::Missing;
                }
            }
        } else if !run && self.capture.take().is_some() {
            self.sink.clear(Feed::Camera);
        }
        // A disabled track sends black frames, like the web's camera turned off
        self.track.set_enabled(self.sending());
    }

    /// Restarts on the new camera at the next [`sync`](Self::sync), without touching the track: no renegotiation.
    pub(crate) fn select(&mut self, id: String) {
        if self.devices.iter().any(|d| d.id == id) {
            self.id = Some(id);
            self.capture = None;
        }
    }

    pub(crate) fn next(&mut self) {
        let current = self.devices.iter().position(|d| Some(&d.id) == self.id.as_ref()).unwrap_or(0);
        if let Some(next) = self.devices.get(current + 1).or(self.devices.first()) {
            self.select(next.id.clone());
        }
    }

    pub(crate) fn set_portrait(&mut self, portrait: bool) {
        self.portrait = portrait;
        self.output.set_portrait(portrait);
    }
}
