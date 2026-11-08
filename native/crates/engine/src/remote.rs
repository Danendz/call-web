//! The other person's video, routed to the camera and screen feeds by the stream it arrives in, like the web's
//! emitRemote: their `media` messages say which stream is the screen; everything else is the camera. Tracks and
//! announcements can come in either order, so each change re-routes.

use std::collections::HashSet;
use std::sync::Arc;

use libwebrtc::video_track::RtcVideoTrack;
use protocol::MAX_STREAM_ID_LENGTH;
use tokio::task::JoinHandle;

use crate::video::{self, Feed, VideoSink};

struct Pump {
    /// The stream and the track in it; a new track in the same stream starts a new pump
    key: (String, String),
    task: JoinHandle<()>,
}

pub(crate) struct RemoteVideos {
    sink: Arc<dyn VideoSink>,
    /// The newest video track of each stream
    tracks: Vec<(String, RtcVideoTrack)>,
    /// Every id ever announced as a screen, so a stopped share's track is never taken for the camera
    screen_ids: HashSet<String>,
    screen_id: Option<String>,
    camera: Option<Pump>,
    screen: Option<Pump>,
}

impl RemoteVideos {
    pub(crate) fn new(sink: Arc<dyn VideoSink>) -> Self {
        Self { sink, tracks: Vec::new(), screen_ids: HashSet::new(), screen_id: None, camera: None, screen: None }
    }

    pub(crate) fn add(&mut self, stream_id: String, track: RtcVideoTrack) {
        self.tracks.retain(|(id, _)| *id != stream_id);
        self.tracks.push((stream_id, track));
        self.route();
    }

    /// Which stream is their screen, from a `media` message; None once they stop sharing. Anything that can't be a
    /// stream id is ignored.
    pub(crate) fn set_screen(&mut self, id: Option<String>) {
        let id = id.filter(|id| id.len() <= MAX_STREAM_ID_LENGTH);
        if let Some(id) = &id {
            self.screen_ids.insert(id.clone());
        }
        self.screen_id = id;
        self.route();
    }

    pub(crate) fn has_camera(&self) -> bool {
        self.camera.is_some()
    }

    pub(crate) fn has_screen(&self) -> bool {
        self.screen.is_some()
    }

    /// The peer connection is gone: its tracks with it. What they announced stays until they leave.
    pub(crate) fn drop_tracks(&mut self) {
        self.tracks.clear();
        self.screen_ids.clear();
        self.route();
    }

    /// They left: forget their screen too.
    pub(crate) fn reset(&mut self) {
        self.screen_id = None;
        self.drop_tracks();
    }

    fn route(&mut self) {
        let ids: Vec<&str> = self.tracks.iter().map(|(id, _)| id.as_str()).collect();
        let (camera, screen) = pick(&ids, &self.screen_ids, self.screen_id.as_deref());
        let camera = camera.and_then(|i| self.tracks.get(i)).cloned();
        let screen = screen.and_then(|i| self.tracks.get(i)).cloned();
        swap(&mut self.camera, camera, Feed::Peer, &self.sink);
        swap(&mut self.screen, screen, Feed::Screen, &self.sink);
    }
}

impl Drop for RemoteVideos {
    fn drop(&mut self) {
        for pump in [self.camera.take(), self.screen.take()].into_iter().flatten() {
            pump.task.abort();
        }
    }
}

/// Which of the streams (oldest first) is their camera and which their screen: the screen is the one they announce,
/// the camera the newest that was never announced as a screen.
fn pick(streams: &[&str], screen_ids: &HashSet<String>, screen_id: Option<&str>) -> (Option<usize>, Option<usize>) {
    let camera = streams.iter().rposition(|id| !screen_ids.contains(*id));
    let screen = screen_id.and_then(|wanted| streams.iter().position(|id| *id == wanted));
    (camera, screen)
}

/// Points a feed at another stream, leaving it alone when it already shows that one.
fn swap(slot: &mut Option<Pump>, wanted: Option<(String, RtcVideoTrack)>, feed: Feed, sink: &Arc<dyn VideoSink>) {
    let key = wanted.as_ref().map(|(id, track)| (id.clone(), track.id()));
    if slot.as_ref().map(|p| &p.key) == key.as_ref() {
        return;
    }
    if let Some(old) = slot.take() {
        old.task.abort();
        sink.clear(feed);
    }
    *slot = wanted.zip(key).map(|((_, track), key)| Pump { key, task: video::pump(track, feed, sink.clone()) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_screen_arriving_before_its_announcement_is_not_taken_for_the_camera_once_announced() {
        // Their screen's track came first and was briefly the newest "camera"
        assert_eq!(pick(&["cam", "scr"], &ids(&[]), None), (Some(1), None));
        assert_eq!(pick(&["cam", "scr"], &ids(&["scr"]), Some("scr")), (Some(0), Some(1)));
    }

    #[test]
    fn a_stopped_share_stays_off_the_camera() {
        assert_eq!(pick(&["cam", "scr"], &ids(&["scr"]), None), (Some(0), None));
    }

    #[test]
    fn an_announced_screen_shows_only_once_its_track_is_here() {
        assert_eq!(pick(&["cam"], &ids(&["scr"]), Some("scr")), (Some(0), None));
    }
}
