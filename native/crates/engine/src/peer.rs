//! One peer connection to the other person using "perfect negotiation", like the web's `peer.ts`, so either side
//! can renegotiate (ICE restart, relay toggle) without offer glare breaking the call. A browser is on the other end,
//! so descriptions and candidates are sent in the shapes `RTCSessionDescription.toJSON()` and
//! `RTCIceCandidate.toJSON()` produce.

use std::time::Duration;

use libwebrtc::MediaType;
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{IceConnectionState, OfferOptions, PeerConnection, SignalingState, TrackEvent};
use libwebrtc::peer_connection_factory::{IceServer, IceTransportsType, PeerConnectionFactory, RtcConfiguration};
use libwebrtc::rtp_parameters::{DegradationPreference, Priority};
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::session_description::{SdpType, SessionDescription};
use libwebrtc::video_track::RtcVideoTrack;
use protocol::{IceServerConfig, SessionDescription as Description, SignalPayload};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, Interval, interval, sleep_until};

use crate::screen::ScreenTracks;
use crate::stats::{self, CallStats, Counters};

const DISCONNECTED_GRACE: Duration = Duration::from_secs(4);
const MIN_RESTART_INTERVAL: Duration = Duration::from_secs(5);
const STATS_EVERY: Duration = Duration::from_secs(1);
// While the stats overlay is hidden only the relay check reads them, and its retries are 15 s apart at the least
const STATS_HIDDEN_EVERY: Duration = Duration::from_secs(5);
// Enough for clean 720p at 30 fps; a direct call into mainland China carried ~2 Mbps with negligible loss
const VIDEO_MAX_BITRATE: u64 = 2_000_000;
// A camera that gives 60 fps gets half again, so its frames aren't blurrier than at 30
const VIDEO_60FPS_MAX_BITRATE: u64 = 3_000_000;
// While sharing: the screen at up to 60 fps, and a small face for the camera at 30. The screen's cap is only a
// ceiling: congestion control finds what the path carries below it, so a LAN or a good link gets a sharp, smooth
// screen (a playing video needs far more than text), and a weak one isn't hurt
const SHARE_SCREEN_BITRATE: u64 = 15_000_000;
const SHARE_SCREEN_FPS: f64 = 60.0;
const SHARE_CAMERA_BITRATE: u64 = 400_000;
const SHARE_CAMERA_FPS: f64 = 30.0;
// The camera's 720p down to the web's 360p short side while sharing
const SHARE_CAMERA_SCALE: f64 = 2.0;
/// The ids of our screen's tracks, to tell their senders from the camera's and the mic's.
pub const SCREEN_TRACK: &str = "screen";
pub const SCREEN_AUDIO_TRACK: &str = "screen-audio";

pub enum PeerEvent {
    Signal(SignalPayload),
    IceState(IceConnectionState),
    /// A video track from the other side, with the id of the stream they sent it in ("" for none), which says
    /// whether it's their camera or their screen
    RemoteVideo {
        stream_id: String,
        track: RtcVideoTrack,
    },
    /// Their voice, or their screen's sound. It plays by itself; this is for muting it, and taking the voice's echo
    /// out of speech to chat
    RemoteAudio {
        track: RtcAudioTrack,
        voice: bool,
    },
    Stats(Box<CallStats>),
}

enum Input {
    Remote(SignalPayload),
    Negotiate,
    Candidate(IceCandidate),
    IceState(IceConnectionState),
    Track(TrackEvent),
    RestartIce,
    ForceRelay(bool),
    Screen(Option<ScreenTracks>),
    StatsShown(bool),
}

pub struct PeerConfig {
    pub ice_servers: Vec<IceServerConfig>,
    pub force_relay: bool,
    pub polite: bool,
    /// The stream id our mic and camera are sent in; the other side groups tracks by it.
    pub stream_id: String,
    /// Our shared screen goes out in its own stream, so the other side can tell it from the camera.
    pub screen_stream_id: String,
    /// The stats overlay is up, so stats are sampled every second instead of every few
    pub stats_shown: bool,
}

/// Runs until dropped; dropping it closes the connection.
pub struct Peer {
    input: mpsc::UnboundedSender<Input>,
    // The connection's own callbacks hold senders for `input` too, so dropping it can't end the session: dropping
    // this does
    _close: oneshot::Sender<()>,
}

impl Peer {
    pub fn start(
        factory: &PeerConnectionFactory,
        config: PeerConfig,
        mic: Option<MediaStreamTrack>,
        camera: MediaStreamTrack,
        events: mpsc::UnboundedSender<PeerEvent>,
    ) -> Result<Self, libwebrtc::RtcError> {
        let pc = factory.create_peer_connection(rtc_config(&config.ice_servers, config.force_relay))?;
        let (input, input_rx) = mpsc::unbounded_channel();

        // Everything comes through one queue, so candidates go out after the description they belong to and
        // remote signals are applied strictly in order
        let tx = input.clone();
        pc.on_negotiation_needed(Some(Box::new(move |_| {
            let _ = tx.send(Input::Negotiate);
        })));
        let tx = input.clone();
        pc.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = tx.send(Input::Candidate(candidate));
        })));
        let tx = input.clone();
        pc.on_ice_connection_state_change(Some(Box::new(move |state| {
            let _ = tx.send(Input::IceState(state));
        })));
        let tx = input.clone();
        pc.on_track(Some(Box::new(move |event| {
            let _ = tx.send(Input::Track(event));
        })));

        let stream_ids = vec![config.stream_id.clone()];
        match mic {
            Some(track) => {
                pc.add_track(track, &stream_ids)?;
            }
            None => {
                pc.add_transceiver_for_media(MediaType::Audio, init(RtpTransceiverDirection::SendRecv, &stream_ids))?;
            }
        }
        prefer_red(factory, &pc);
        // add_track, not add_transceiver: the other side's offer can then reuse this transceiver instead of adding one
        pc.add_track(camera, &stream_ids)?;

        let session = Session {
            pc,
            polite: config.polite,
            ice_servers: config.ice_servers,
            screen_stream_id: config.screen_stream_id,
            screen_added: false,
            sharing: false,
            voice_seen: false,
            counters: None,
            ignore_offer: false,
            last_restart: None,
            disconnected_since: None,
            stats: stats_interval(config.stats_shown),
            events,
        };
        let (close, closed) = oneshot::channel();
        tokio::spawn(session.run(input_rx, closed));
        Ok(Self { input, _close: close })
    }

    pub fn handle_signal(&self, payload: SignalPayload) {
        let _ = self.input.send(Input::Remote(payload));
    }

    pub fn restart_ice(&self) {
        let _ = self.input.send(Input::RestartIce);
    }

    /// Sends everything through our TURN server, for testing the relay path from the stats overlay.
    pub fn set_force_relay(&self, relay: bool) {
        let _ = self.input.send(Input::ForceRelay(relay));
    }

    /// Sends the shared screen alongside the camera, or stops; see [`Session::set_screen`].
    pub fn set_screen(&self, tracks: Option<ScreenTracks>) {
        let _ = self.input.send(Input::Screen(tracks));
    }

    pub fn set_stats_shown(&self, shown: bool) {
        let _ = self.input.send(Input::StatsShown(shown));
    }
}

/// Puts RED before Opus on the mic (the first transceiver), so each packet also carries the previous one and a single
/// lost packet leaves no gap. The answer keeps only what both sides have, so a peer without RED gets plain Opus.
fn prefer_red(factory: &PeerConnectionFactory, pc: &PeerConnection) {
    let mut codecs = factory.get_rtp_receiver_capabilities(MediaType::Audio).codecs;
    let is_red = |mime: &str| mime.eq_ignore_ascii_case("audio/red");
    if !codecs.iter().any(|c| is_red(&c.mime_type)) {
        return;
    }
    codecs.sort_by_key(|c| match c.mime_type.as_str() {
        m if is_red(m) => 0,
        m if m.eq_ignore_ascii_case("audio/opus") => 1,
        _ => 2,
    });
    let Some(mic) = pc.transceivers().into_iter().next() else { return };
    if let Err(e) = mic.set_codec_preferences(codecs) {
        tracing::warn!("couldn't prefer RED: {}", e.message);
    }
}

fn init(direction: RtpTransceiverDirection, stream_ids: &[String]) -> RtpTransceiverInit {
    RtpTransceiverInit { direction, stream_ids: stream_ids.to_vec(), send_encodings: vec![] }
}

fn rtc_config(servers: &[IceServerConfig], relay: bool) -> RtcConfiguration {
    let mut config = RtcConfiguration::default();
    config.ice_servers = servers
        .iter()
        .map(|s| IceServer {
            urls: s.urls.clone(),
            username: s.username.clone().unwrap_or_default(),
            password: s.credential.clone().unwrap_or_default(),
        })
        .collect();
    config.ice_transport_type = if relay { IceTransportsType::Relay } else { IceTransportsType::All };
    config
}

fn stats_interval(shown: bool) -> Interval {
    interval(if shown { STATS_EVERY } else { STATS_HIDDEN_EVERY })
}

struct Session {
    pc: PeerConnection,
    polite: bool,
    ice_servers: Vec<IceServerConfig>,
    screen_stream_id: String,
    /// The screen's transceivers are in this connection; they keep their tracks after a share stops
    screen_added: bool,
    sharing: bool,
    /// Their voice's track has arrived
    voice_seen: bool,
    counters: Option<Counters>,
    ignore_offer: bool,
    last_restart: Option<Instant>,
    disconnected_since: Option<Instant>,
    stats: Interval,
    events: mpsc::UnboundedSender<PeerEvent>,
}

impl Session {
    async fn run(mut self, mut input: mpsc::UnboundedReceiver<Input>, mut closed: oneshot::Receiver<()>) {
        loop {
            let grace = self.disconnected_since.map(|since| since + DISCONNECTED_GRACE);
            tokio::select! {
                // The Peer was dropped
                _ = &mut closed => break,
                Some(msg) = input.recv() => self.handle(msg).await,
                _ = self.stats.tick() => self.sample_stats().await,
                () = sleep_until(grace.unwrap_or_else(Instant::now)), if grace.is_some() => {
                    self.disconnected_since = None;
                    self.restart_ice();
                }
            }
        }
        self.pc.on_negotiation_needed(None);
        self.pc.on_ice_candidate(None);
        self.pc.on_ice_connection_state_change(None);
        self.pc.on_track(None);
        self.pc.close();
    }

    async fn handle(&mut self, msg: Input) {
        match msg {
            Input::Remote(payload) => {
                if let Err(e) = self.apply(payload).await {
                    tracing::warn!("signal error: {e}");
                }
            }
            Input::Negotiate => self.negotiate().await,
            Input::Candidate(candidate) => self.emit(PeerEvent::Signal(SignalPayload::Candidate {
                candidate: json!({
                    "candidate": candidate.candidate(),
                    "sdpMid": candidate.sdp_mid(),
                    "sdpMLineIndex": candidate.sdp_mline_index(),
                }),
            })),
            Input::IceState(state) => {
                self.disconnected_since = (state == IceConnectionState::Disconnected).then(Instant::now);
                if state == IceConnectionState::Failed {
                    self.restart_ice();
                }
                self.emit(PeerEvent::IceState(state));
            }
            Input::Track(event) => match event.track {
                MediaStreamTrack::Video(track) => {
                    let stream_id = event.streams.first().map(|s| s.id()).unwrap_or_default();
                    self.emit(PeerEvent::RemoteVideo { stream_id, track });
                }
                // Their voice comes first (its transceiver is the first one); a later one is their screen's sound
                MediaStreamTrack::Audio(track) => {
                    let voice = !self.voice_seen;
                    self.voice_seen = true;
                    self.emit(PeerEvent::RemoteAudio { track, voice });
                }
            },
            Input::RestartIce => self.restart_ice(),
            Input::Screen(track) => self.set_screen(track),
            Input::ForceRelay(relay) => {
                if let Err(e) = self.pc.set_configuration(rtc_config(&self.ice_servers, relay)) {
                    tracing::warn!("couldn't change the relay policy: {}", e.message);
                }
                self.last_restart = None;
                self.restart_ice();
            }
            // Its first tick is now, so the overlay fills as it opens
            Input::StatsShown(shown) => self.stats = stats_interval(shown),
        }
    }

    /// Like the browser's `onnegotiationneeded` + `setLocalDescription()`: only from a stable state, and the
    /// event fires again once the state is stable if it's still needed.
    async fn negotiate(&mut self) {
        if self.pc.signaling_state() != SignalingState::Stable {
            return;
        }
        // Leave offerToReceive unset: the defaults would turn our receive-only transceivers inactive
        let options = OfferOptions { ice_restart: false, offer_to_receive_audio: None, offer_to_receive_video: None };
        let result = async {
            let offer = self.pc.create_offer(options).await?;
            self.pc.set_local_description(offer.clone()).await?;
            Ok::<_, libwebrtc::RtcError>(offer)
        }
        .await;
        match result {
            Ok(offer) => {
                self.send_description(&offer);
                self.tune_senders();
            }
            Err(e) => tracing::warn!("negotiation failed: {}", e.message),
        }
    }

    async fn apply(&mut self, payload: SignalPayload) -> Result<(), String> {
        match payload {
            SignalPayload::Description { description } => {
                let kind: SdpType = description.kind.parse().map_err(|e: &str| e.to_owned())?;
                let sdp = SessionDescription::parse(description.sdp.as_deref().unwrap_or_default(), kind)
                    .map_err(|e| format!("bad sdp: {e:?}"))?;
                let collision = kind == SdpType::Offer && self.pc.signaling_state() != SignalingState::Stable;
                self.ignore_offer = !self.polite && collision;
                if self.ignore_offer {
                    return Ok(());
                }
                if collision {
                    // The polite side drops its own offer for theirs; browsers do this rollback implicitly
                    let rollback = SessionDescription::parse("", SdpType::Rollback).map_err(|e| format!("{e:?}"))?;
                    self.pc.set_local_description(rollback).await.map_err(|e| e.message)?;
                }
                self.pc.set_remote_description(sdp).await.map_err(|e| e.message)?;
                if kind == SdpType::Offer {
                    let answer = self.pc.create_answer(Default::default()).await.map_err(|e| e.message)?;
                    self.pc.set_local_description(answer.clone()).await.map_err(|e| e.message)?;
                    self.send_description(&answer);
                }
                self.tune_senders();
            }
            SignalPayload::Candidate { candidate } => {
                let Some(candidate) = parse_candidate(&candidate) else { return Ok(()) };
                if let Err(e) = self.pc.add_ice_candidate(candidate).await
                    && !self.ignore_offer
                {
                    return Err(e.message);
                }
            }
        }
        Ok(())
    }

    fn send_description(&self, desc: &SessionDescription) {
        self.emit(PeerEvent::Signal(SignalPayload::Description {
            description: Description { kind: desc.sdp_type().to_string(), sdp: Some(desc.to_string()) },
        }));
    }

    /// The first share adds the screen's video and sound transceivers (one renegotiation), like the web's setScreen.
    /// They keep their tracks for the connection's life: a stopped share sends nothing because its capture has
    /// stopped, so sharing again needs no change. (Taking a track out would crash: LiveKit's `set_track(None)`
    /// dereferences the missing track.)
    fn set_screen(&mut self, tracks: Option<ScreenTracks>) {
        self.sharing = tracks.is_some();
        if let Some(ScreenTracks { video, audio }) = tracks
            && !self.screen_added
        {
            self.screen_added = true;
            for track in [video, audio] {
                let init = init(RtpTransceiverDirection::SendOnly, std::slice::from_ref(&self.screen_stream_id));
                if let Err(e) = self.pc.add_transceiver(track, init) {
                    tracing::warn!("couldn't add the screen: {}", e.message);
                }
            }
        }
        self.tune_senders();
    }

    /// Like the web's tuneSenders: caps video bitrate and tells the encoder what to give up when bandwidth runs
    /// short: resolution for the camera (a smaller clean picture beats a blocky one), some of both for the screen (a
    /// playing video stays watchable instead of freezing, and text is sharp again once it settles). While sharing,
    /// the camera shrinks to a small face so the screen gets the bandwidth. The mic gets priority so voices survive
    /// congestion. Encodings only exist after negotiation, so this runs after every description change.
    fn tune_senders(&self) {
        let sharing = self.sharing;
        let camera_60 = crate::camera::fps() > 30;
        for transceiver in self.pc.transceivers() {
            let sender = transceiver.sender();
            let (video, screen) = match sender.track() {
                Some(MediaStreamTrack::Video(track)) => (true, track.id() == SCREEN_TRACK),
                // The screen's sound keeps the default priority, below the voice
                Some(MediaStreamTrack::Audio(track)) if track.id() == SCREEN_AUDIO_TRACK => continue,
                Some(MediaStreamTrack::Audio(_)) => (false, false),
                None => continue,
            };
            let mut params = sender.parameters();
            if params.encodings.is_empty() {
                continue;
            }
            let (max_bitrate, max_framerate) = match (screen, sharing) {
                (true, _) => (SHARE_SCREEN_BITRATE, Some(SHARE_SCREEN_FPS)),
                (false, true) => (SHARE_CAMERA_BITRATE, Some(SHARE_CAMERA_FPS)),
                (false, false) if camera_60 => (VIDEO_60FPS_MAX_BITRATE, None),
                (false, false) => (VIDEO_MAX_BITRATE, None),
            };
            for enc in &mut params.encodings {
                if video {
                    enc.max_bitrate = Some(max_bitrate);
                    enc.max_framerate = max_framerate;
                    enc.scale_resolution_down_by = Some(if sharing && !screen { SHARE_CAMERA_SCALE } else { 1.0 });
                } else {
                    enc.priority = Priority::High;
                }
            }
            if video {
                params.set_degradation_preference(if screen {
                    DegradationPreference::Balanced
                } else {
                    DegradationPreference::MaintainFramerate
                });
            }
            if let Err(e) = sender.set_parameters(params) {
                tracing::debug!("couldn't tune a sender: {}", e.message);
            }
        }
    }

    async fn sample_stats(&mut self) {
        let Ok(report) = self.pc.get_stats().await else { return };
        let (stats, counters) = stats::sample(&report, self.counters.as_ref(), Instant::now().into_std());
        self.counters = Some(counters);
        self.emit(PeerEvent::Stats(Box::new(stats)));
    }

    fn restart_ice(&mut self) {
        if self.last_restart.is_some_and(|at| at.elapsed() < MIN_RESTART_INTERVAL) {
            return;
        }
        self.last_restart = Some(Instant::now());
        self.pc.restart_ice();
    }

    fn emit(&self, event: PeerEvent) {
        let _ = self.events.send(event);
    }
}

/// A browser's `RTCIceCandidateInit`; null or an empty candidate marks the end of gathering, which needs nothing.
fn parse_candidate(value: &Value) -> Option<IceCandidate> {
    let candidate = value.get("candidate")?.as_str().filter(|c| !c.is_empty())?;
    let mid = value.get("sdpMid").and_then(Value::as_str).unwrap_or_default();
    let index = value.get("sdpMLineIndex").and_then(Value::as_i64).and_then(|i| i32::try_from(i).ok()).unwrap_or(0);
    match IceCandidate::parse(mid, index, candidate) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::debug!("ignoring an unreadable candidate: {e:?}");
            None
        }
    }
}
