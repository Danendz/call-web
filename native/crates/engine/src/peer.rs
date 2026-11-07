//! One peer connection to the other person using "perfect negotiation", like the web's `peer.ts`, so either side
//! can renegotiate (ICE restart, relay toggle) without offer glare breaking the call. A browser is on the other end,
//! so descriptions and candidates are sent in the shapes `RTCSessionDescription.toJSON()` and
//! `RTCIceCandidate.toJSON()` produce.

use std::time::Duration;

use libwebrtc::MediaType;
use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{IceConnectionState, OfferOptions, PeerConnection, SignalingState};
use libwebrtc::peer_connection_factory::{IceServer, PeerConnectionFactory, RtcConfiguration};
use libwebrtc::rtp_parameters::Priority;
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::session_description::{SdpType, SessionDescription};
use protocol::{IceServerConfig, SessionDescription as Description, SignalPayload};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};

const DISCONNECTED_GRACE: Duration = Duration::from_secs(4);
const MIN_RESTART_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum PeerEvent {
    Signal(SignalPayload),
    IceState(IceConnectionState),
}

enum Input {
    Remote(SignalPayload),
    Negotiate,
    Candidate(IceCandidate),
    IceState(IceConnectionState),
    RestartIce,
}

pub struct PeerConfig {
    pub ice_servers: Vec<IceServerConfig>,
    pub polite: bool,
    /// The stream id our mic (and later camera) is sent in; the other side groups tracks by it.
    pub stream_id: String,
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
        events: mpsc::UnboundedSender<PeerEvent>,
    ) -> Result<Self, libwebrtc::RtcError> {
        let pc = factory.create_peer_connection(rtc_config(&config.ice_servers))?;
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

        let stream_ids = vec![config.stream_id.clone()];
        match mic {
            Some(track) => {
                pc.add_track(track, &stream_ids)?;
            }
            None => {
                pc.add_transceiver_for_media(MediaType::Audio, init(RtpTransceiverDirection::SendRecv, &stream_ids))?;
            }
        }

        let session = Session {
            pc,
            polite: config.polite,
            ignore_offer: false,
            last_restart: None,
            disconnected_since: None,
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
}

fn init(direction: RtpTransceiverDirection, stream_ids: &[String]) -> RtpTransceiverInit {
    RtpTransceiverInit { direction, stream_ids: stream_ids.to_vec(), send_encodings: vec![] }
}

fn rtc_config(servers: &[IceServerConfig]) -> RtcConfiguration {
    let mut config = RtcConfiguration::default();
    config.ice_servers = servers
        .iter()
        .map(|s| IceServer {
            urls: s.urls.clone(),
            username: s.username.clone().unwrap_or_default(),
            password: s.credential.clone().unwrap_or_default(),
        })
        .collect();
    config
}

struct Session {
    pc: PeerConnection,
    polite: bool,
    ignore_offer: bool,
    last_restart: Option<Instant>,
    disconnected_since: Option<Instant>,
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
                () = sleep_until(grace.unwrap_or_else(Instant::now)), if grace.is_some() => {
                    self.disconnected_since = None;
                    self.restart_ice();
                }
            }
        }
        self.pc.on_negotiation_needed(None);
        self.pc.on_ice_candidate(None);
        self.pc.on_ice_connection_state_change(None);
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
            Input::RestartIce => self.restart_ice(),
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

    /// The mic gets priority so voices survive congestion. Video caps arrive with the camera.
    fn tune_senders(&self) {
        for transceiver in self.pc.transceivers() {
            let sender = transceiver.sender();
            if !matches!(sender.track(), Some(MediaStreamTrack::Audio(_))) {
                continue;
            }
            let mut params = sender.parameters();
            if params.encodings.is_empty() {
                continue;
            }
            for enc in &mut params.encodings {
                enc.priority = Priority::High;
            }
            if let Err(e) = sender.set_parameters(params) {
                tracing::debug!("couldn't raise the mic's priority: {}", e.message);
            }
        }
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
