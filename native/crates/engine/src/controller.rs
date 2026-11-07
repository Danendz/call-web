//! Owns the call's state and everything that changes it, on its own tokio runtime. The UI only sends
//! [`Command`]s and renders each [`State`] it's handed. Mirrors the web's `controller.ts`.

use std::thread;
use std::time::{Duration, Instant};

use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::IceConnectionState;
use protocol::{ClientMessage, IceServerConfig, Lang, ServerMessage};
use tokio::sync::mpsc;
use url::Url;

use crate::level::{Level, Meter};
use crate::media::{Device, Media};
use crate::peer::{Peer, PeerConfig, PeerEvent};
use crate::room;
use crate::signaling::{SignalEvent, Signaling, SocketStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No room picked yet: new room or join by link
    Start,
    /// Room picked, not joined: the pre-join screen
    Setup,
    Starting,
    /// Joined, alone in the room
    Waiting,
    /// The other person is here and a peer connection to them exists
    InCall,
    Full,
    Ended,
}

/// The media path to the other person, as the call screen's banner shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    /// Dropped for now; an ICE restart follows if it doesn't come back by itself
    Unstable,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub phase: Phase,
    pub lang: Lang,
    pub room_id: Option<String>,
    /// The web address of the room, for sharing: it opens in a browser or in this app.
    pub room_url: Option<String>,
    /// The socket dropped and is being reopened; the seat is kept for a while meanwhile.
    pub reconnecting: bool,
    pub link: Option<Link>,
    /// When the call with the other person began, on this machine's clock; None while alone.
    pub call_started: Option<Instant>,
    pub mic_on: bool,
    pub mics: Vec<Device>,
    pub mic_id: Option<String>,
    /// The other person's camera is on (from their `media` messages).
    pub remote_cam_on: bool,
}

#[derive(Debug, Clone)]
pub enum Command {
    NewRoom,
    /// A room id, or a link someone pasted, already checked with [`room::parse_room`].
    OpenRoom(String),
    SetLang(Lang),
    Join,
    HangUp,
    /// Back to the start screen
    Home,
    ToggleMic,
    /// Only before joining, as on the web: switching the mic mid-call would restart capture under the call.
    SelectMic(String),
}

/// Handle to the engine thread. Dropping it leaves the room and stops the thread.
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    level: Level,
}

impl Engine {
    /// `on_state` runs on the engine's thread for every change; the UI hops to its own thread from there.
    pub fn start(server: Url, lang: Lang, on_state: impl Fn(State) + Send + 'static) -> std::io::Result<Self> {
        let (commands, rx) = mpsc::unbounded_channel();
        let level = Level::default();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let controller_level = level.clone();
        thread::Builder::new().name("engine".into()).spawn(move || {
            runtime.block_on(Controller::new(server, lang, controller_level, Box::new(on_state)).run(rx));
        })?;
        Ok(Self { commands, level })
    }

    /// The mic level on the setup screen, 0..=1; 0 elsewhere. Cheap enough to read every frame.
    pub fn mic_level(&self) -> f32 {
        self.level.get()
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// What the server told us when we joined, needed again for every new peer connection.
#[derive(Default)]
struct Seat {
    polite: bool,
    ice_servers: Vec<IceServerConfig>,
}

struct Controller {
    server: Url,
    state: State,
    on_state: Box<dyn Fn(State) + Send>,
    media: Media,
    level: Level,
    /// Only while the setup screen is up
    meter: Option<Meter>,
    /// Our side of the call: the mic track and the stream id it's sent in, kept across peers
    mic: Option<MediaStreamTrack>,
    stream_id: String,
    signaling: Option<Signaling>,
    signal_tx: mpsc::UnboundedSender<SignalEvent>,
    signal_rx: mpsc::UnboundedReceiver<SignalEvent>,
    seat: Seat,
    peer: Option<Peer>,
    peer_tx: mpsc::UnboundedSender<PeerEvent>,
    peer_rx: mpsc::UnboundedReceiver<PeerEvent>,
}

impl Controller {
    fn new(server: Url, lang: Lang, level: Level, on_state: Box<dyn Fn(State) + Send>) -> Self {
        let (signal_tx, signal_rx) = mpsc::unbounded_channel();
        let (peer_tx, peer_rx) = mpsc::unbounded_channel();
        let media = Media::new();
        let state = State {
            phase: Phase::Start,
            lang,
            room_id: None,
            room_url: None,
            reconnecting: false,
            link: None,
            call_started: None,
            mic_on: true,
            mics: media.mics(),
            mic_id: None,
            remote_cam_on: true,
        };
        Self {
            server,
            state,
            on_state,
            media,
            level,
            meter: None,
            mic: None,
            stream_id: room::random_id(),
            signaling: None,
            signal_tx,
            signal_rx,
            seat: Seat::default(),
            peer: None,
            peer_tx,
            peer_rx,
        }
    }

    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.publish();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => {
                        self.command(command);
                        self.sync_meter();
                    }
                    None => break,
                },
                Some(event) = self.signal_rx.recv() => self.signal_event(event),
                Some(event) = self.peer_rx.recv() => self.peer_event(event),
            }
            self.publish();
        }
        if let Some(signaling) = self.signaling.take() {
            signaling.leave();
            // Give the leave a moment to go out before the runtime shuts down
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::NewRoom => self.open_room(room::new_room_id()),
            Command::OpenRoom(id) if protocol::is_room_id(&id) => self.open_room(id),
            Command::OpenRoom(_) => tracing::warn!("ignoring an invalid room id"),
            Command::SetLang(lang) => self.state.lang = lang,
            Command::Join => self.join(),
            Command::HangUp => {
                self.disconnect();
                self.state.phase = Phase::Ended;
            }
            Command::Home => {
                self.disconnect();
                self.state.phase = Phase::Start;
                self.state.room_id = None;
                self.state.room_url = None;
            }
            Command::ToggleMic => {
                self.state.mic_on = !self.state.mic_on;
                if let Some(mic) = &self.mic {
                    mic.set_enabled(self.state.mic_on);
                }
            }
            Command::SelectMic(id) => {
                if self.mic.is_none() && self.media.select_mic(&id) {
                    self.state.mic_id = Some(id);
                    // Reopened on the new mic
                    self.meter = None;
                }
            }
        }
    }

    fn sync_meter(&mut self) {
        if self.state.phase != Phase::Setup || self.state.mics.is_empty() {
            self.meter = None;
        } else if self.meter.is_none() {
            let id = self.state.mic_id.as_ref();
            let name = self.state.mics.iter().find(|d| Some(&d.id) == id).map(|d| d.label.clone());
            self.meter = Some(Meter::start(name, self.level.clone()));
        }
    }

    fn open_room(&mut self, id: String) {
        self.disconnect();
        self.state.room_url = Some(room::room_url(&self.server, &id));
        self.state.room_id = Some(id);
        self.state.phase = Phase::Setup;
        self.state.mics = self.media.mics();
    }

    fn join(&mut self) {
        let Some(room_id) = self.state.room_id.clone() else { return };
        self.disconnect();
        // Joining without a mic is allowed: you can still hear and chat
        self.mic = self.media.mic_track().map(MediaStreamTrack::Audio);
        if let Some(mic) = &self.mic {
            mic.set_enabled(self.state.mic_on);
        }
        self.signaling = Some(Signaling::connect(&self.server, &room_id, self.signal_tx.clone()));
        self.state.phase = Phase::Starting;
    }

    fn disconnect(&mut self) {
        self.drop_peer();
        if let Some(signaling) = self.signaling.take() {
            signaling.leave();
        }
        // Events from the old socket still queued must not touch the next call
        while self.signal_rx.try_recv().is_ok() {}
        self.mic = None;
        self.seat = Seat::default();
        self.state.reconnecting = false;
        self.state.remote_cam_on = true;
    }

    fn signal_event(&mut self, event: SignalEvent) {
        match event {
            SignalEvent::Status(status) => self.state.reconnecting = status == SocketStatus::Reconnecting,
            SignalEvent::Message(msg) => self.server_message(msg),
        }
    }

    fn server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Joined {
                client_id,
                token,
                resumed,
                peer_present,
                polite,
                ice_servers,
                call_elapsed_ms,
                ..
            } => {
                if let Some(signaling) = &self.signaling {
                    signaling.set_identity(client_id, token);
                }
                self.seat.ice_servers = ice_servers;
                if resumed && self.peer.is_some() {
                    // Our socket dropped (network switch); the media path likely died with it
                    if peer_present {
                        if let Some(peer) = &self.peer {
                            peer.restart_ice();
                        }
                        self.send_media(); // in case an update was lost with the socket
                    } else {
                        self.drop_peer();
                    }
                } else {
                    self.seat.polite = polite;
                    if peer_present {
                        self.create_peer();
                    }
                }
                if self.peer.is_some() {
                    self.set_call_elapsed(call_elapsed_ms);
                } else {
                    self.state.phase = Phase::Waiting;
                }
            }
            ServerMessage::PeerJoined { polite, call_elapsed_ms } => {
                self.seat.polite = polite;
                self.create_peer();
                self.set_call_elapsed(Some(call_elapsed_ms));
            }
            ServerMessage::PeerLeft => {
                self.drop_peer();
                self.state.remote_cam_on = true;
            }
            ServerMessage::Signal { payload } => {
                if let Some(peer) = &self.peer {
                    peer.handle_signal(payload);
                }
            }
            ServerMessage::Media { cam_on, .. } => self.state.remote_cam_on = cam_on,
            ServerMessage::Full => {
                self.signaling = None;
                self.drop_peer();
                self.state.phase = Phase::Full;
            }
            ServerMessage::Error { message } => tracing::warn!("server error: {message}"),
            // Chat and the rest arrive with the milestones that use them
            _ => {}
        }
    }

    fn create_peer(&mut self) {
        self.drop_peer();
        let config = PeerConfig {
            ice_servers: self.seat.ice_servers.clone(),
            polite: self.seat.polite,
            stream_id: self.stream_id.clone(),
        };
        match Peer::start(self.media.factory(), config, self.mic.clone(), self.peer_tx.clone()) {
            Ok(peer) => {
                self.peer = Some(peer);
                self.state.phase = Phase::InCall;
                self.state.link = Some(Link::Connecting);
                self.send_media();
            }
            Err(e) => {
                tracing::error!("couldn't create the peer connection: {}", e.message);
                self.state.phase = Phase::Waiting;
            }
        }
    }

    fn drop_peer(&mut self) {
        self.peer = None;
        // Events from the old connection still queued must not touch the next one
        while self.peer_rx.try_recv().is_ok() {}
        self.state.link = None;
        self.state.call_started = None;
        if self.state.phase == Phase::InCall {
            self.state.phase = Phase::Waiting;
        }
    }

    fn set_call_elapsed(&mut self, ms: Option<u64>) {
        let Some(ms) = ms else { return };
        let now = Instant::now();
        self.state.call_started = Some(now.checked_sub(Duration::from_millis(ms)).unwrap_or(now));
    }

    /// Tells the other side what we're sending, which the tracks alone can't say. No camera yet.
    fn send_media(&self) {
        if self.peer.is_none() {
            return;
        }
        if let Some(signaling) = &self.signaling {
            signaling.send(ClientMessage::Media { cam_on: false, screen: None });
        }
    }

    fn peer_event(&mut self, event: PeerEvent) {
        match event {
            PeerEvent::Signal(payload) => {
                if let Some(signaling) = &self.signaling {
                    signaling.send(ClientMessage::Signal { payload });
                }
            }
            PeerEvent::IceState(ice) => {
                self.state.link = Some(match ice {
                    IceConnectionState::Connected | IceConnectionState::Completed => Link::Connected,
                    IceConnectionState::Disconnected => Link::Unstable,
                    IceConnectionState::Failed => Link::Failed,
                    _ => Link::Connecting,
                });
            }
        }
    }

    fn publish(&self) {
        (self.on_state)(self.state.clone());
    }
}
