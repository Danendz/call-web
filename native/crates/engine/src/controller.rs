//! Owns the call's state and everything that changes it, on its own tokio runtime. The UI only sends
//! [`Command`]s and renders each [`State`] it's handed. Mirrors the web's `controller.ts`.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::IceConnectionState;
use protocol::{
    ChatMessage, ClientMessage, IceServerConfig, Lang, MAX_CHAT_LENGTH, ServerMessage, SpeechServerMessage, Translation,
};
use tokio::sync::mpsc;
use url::Url;

use crate::camera::CameraAccess;
use crate::camera::local::LocalCamera;
use crate::level::{Level, Meter};
use crate::media::{Device, Media};
use crate::peer::{Peer, PeerConfig, PeerEvent};
use crate::room;
use crate::signaling::{SignalEvent, Signaling, SocketStatus};
use crate::speech::{Params as SpeechParams, SpeechNotice, Transcriber};
use crate::stats::{CallStats, Path};
use crate::video::{self, Feed, VideoSink};

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
    /// Their video track has arrived; frames go to the sink as [`Feed::Peer`]
    pub remote_video: bool,
    pub cam_on: bool,
    pub cam_access: CameraAccess,
    pub cams: Vec<Device>,
    pub cam_id: Option<String>,
    /// We send a 9:16 picture instead of 16:9
    pub portrait: bool,
    pub stats: Option<CallStats>,
    /// The other person just saved a picture of the call
    pub peer_screenshot: bool,
    /// Speech to chat is on: what you say is posted to the chat while someone's there to read it
    pub transcribe_on: bool,
    pub speech_notice: Option<SpeechNotice>,
    /// Everything goes through our TURN server (a stats overlay switch, for testing the relay)
    pub force_relay: bool,
    /// An ICE restart to get off the relay just went out
    pub trying_direct: bool,
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
    ToggleCam,
    /// Any time: the new camera feeds the same track, so a call needs no renegotiation
    SelectCam(String),
    /// The next camera in the list: a phone's front and back, or a laptop's two
    NextCam,
    SetPortrait(bool),
    SetForceRelay(bool),
    /// Sent as typed, trimmed; nothing if it's empty or longer than the server takes
    SendChat(String),
    /// Tells the other person a picture of the call was saved
    NotifyScreenshot,
    SetTranscribe(bool),
}

/// The chat, as changes: a whole new list on joining, then single messages and translations, so the UI updates its
/// list in place. Kept out of [`State`], which would otherwise copy every message on every change.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    Reset { my_id: String, messages: Vec<ChatMessage> },
    Added(ChatMessage),
    Translated { id: String, translation: Translation },
}

// While a call is relayed, retry direct after these delays (the last one repeats). Direct is ~2x faster than going
// through Singapore, and a P2P attempt that failed at call start often works a few minutes later.
const DIRECT_RETRY_DELAYS: [Duration; 4] =
    [Duration::from_secs(15), Duration::from_secs(30), Duration::from_secs(60), Duration::from_secs(120)];
const TRYING_DIRECT_SHOWN: Duration = Duration::from_secs(5);

const PEER_SCREENSHOT_SHOWN: Duration = Duration::from_secs(3);

/// What the engine tells itself from other threads and timers.
enum Internal {
    CameraAccess(CameraAccess),
    PeerScreenshotShown,
    /// Speech to chat's quota is back
    SpeechResumed,
}

/// Handle to the engine thread. Dropping it leaves the room and stops the thread.
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    level: Level,
}

impl Engine {
    /// `on_state` runs on the engine's thread for every change; the UI hops to its own thread from there.
    /// Video frames go to `sink`, from video threads; see [`VideoSink`].
    pub fn start(
        server: Url,
        lang: Lang,
        sink: Arc<dyn VideoSink>,
        on_state: impl Fn(State) + Send + 'static,
        on_chat: impl Fn(ChatEvent) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, rx) = mpsc::unbounded_channel();
        let level = Level::default();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let controller_level = level.clone();
        thread::Builder::new().name("engine".into()).spawn(move || {
            // Built inside the runtime: WebRTC's video source needs one to exist
            runtime.block_on(async move {
                Controller::new(server, lang, controller_level, sink, Box::new(on_state), Box::new(on_chat))
                    .run(rx)
                    .await;
            });
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
    /// Secret for resuming the seat and for `/ws/speech`; never logged
    token: Option<String>,
    ice_servers: Vec<IceServerConfig>,
}

struct Controller {
    server: Url,
    state: State,
    on_state: Box<dyn Fn(State) + Send>,
    on_chat: Box<dyn Fn(ChatEvent) + Send>,
    media: Media,
    level: Level,
    /// Only while the setup screen is up
    meter: Option<Meter>,
    /// Our side of the call: the mic track and the stream id it's sent in, kept across peers
    mic: Option<MediaStreamTrack>,
    camera: LocalCamera,
    sink: Arc<dyn VideoSink>,
    /// Shows the other person's video while it lasts
    remote_video: Option<tokio::task::JoinHandle<()>>,
    internal_tx: mpsc::UnboundedSender<Internal>,
    internal_rx: mpsc::UnboundedReceiver<Internal>,
    transcriber: Transcriber,
    speech_rx: mpsc::UnboundedReceiver<SpeechServerMessage>,
    direct_retries: usize,
    next_direct_retry: Option<Instant>,
    last_direct_retry: Option<Instant>,
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
    fn new(
        server: Url,
        lang: Lang,
        level: Level,
        sink: Arc<dyn VideoSink>,
        on_state: Box<dyn Fn(State) + Send>,
        on_chat: Box<dyn Fn(ChatEvent) + Send>,
    ) -> Self {
        let (signal_tx, signal_rx) = mpsc::unbounded_channel();
        let (peer_tx, peer_rx) = mpsc::unbounded_channel();
        let (internal_tx, internal_rx) = mpsc::unbounded_channel();
        let (speech_tx, speech_rx) = mpsc::unbounded_channel();
        let media = Media::new();
        let camera = LocalCamera::new(media.factory(), sink.clone());
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
            remote_video: false,
            cam_on: true,
            cam_access: CameraAccess::Pending,
            cams: Vec::new(),
            cam_id: None,
            portrait: false,
            stats: None,
            force_relay: false,
            trying_direct: false,
            peer_screenshot: false,
            transcribe_on: false,
            speech_notice: None,
        };
        Self {
            server,
            state,
            on_state,
            on_chat,
            media,
            level,
            meter: None,
            mic: None,
            camera,
            sink,
            remote_video: None,
            internal_tx,
            internal_rx,
            transcriber: Transcriber::new(speech_tx),
            speech_rx,
            direct_retries: 0,
            next_direct_retry: None,
            last_direct_retry: None,
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
                    Some(command) => self.command(command),
                    None => break,
                },
                Some(event) = self.signal_rx.recv() => self.signal_event(event),
                Some(event) = self.peer_rx.recv() => self.peer_event(event),
                Some(internal) = self.internal_rx.recv() => match internal {
                    Internal::CameraAccess(access) => self.camera.set_access(access),
                    Internal::PeerScreenshotShown => self.state.peer_screenshot = false,
                    Internal::SpeechResumed => {
                        if matches!(self.state.speech_notice, Some(SpeechNotice::Paused { .. })) {
                            self.state.speech_notice = None;
                        }
                    }
                },
                Some(msg) = self.speech_rx.recv() => self.speech_notice(msg),
            }
            self.sync_devices();
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
            Command::ToggleCam => {
                self.camera.on = !self.camera.on;
                // The light goes off at once; the other side hears about it after the sync below
                self.camera.sync(self.camera_wanted());
                self.send_media();
            }
            Command::SelectCam(id) => self.camera.select(id),
            Command::NextCam => self.camera.next(),
            Command::SetPortrait(portrait) => self.camera.set_portrait(portrait),
            Command::SendChat(text) => {
                let text = text.trim();
                // The server counts UTF-16 units, like JavaScript's String.length
                if !text.is_empty()
                    && text.encode_utf16().count() <= MAX_CHAT_LENGTH
                    && let Some(signaling) = &self.signaling
                {
                    signaling.send(ClientMessage::Chat { text: text.to_owned(), spoken: None });
                }
            }
            Command::SetTranscribe(on) => self.state.transcribe_on = on,
            Command::NotifyScreenshot => {
                if let Some(signaling) = &self.signaling {
                    signaling.send(ClientMessage::Screenshot);
                }
            }
            Command::SetForceRelay(relay) => {
                self.state.force_relay = relay;
                if let Some(peer) = &self.peer {
                    peer.set_force_relay(relay);
                }
            }
        }
    }

    /// Starts and stops what depends on the screen: the mic meter on setup, the camera while a screen shows it or a
    /// call sends it. Runs after everything, so no path forgets it.
    fn sync_devices(&mut self) {
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
        let speaking = self.state.transcribe_on && self.state.mic_on && self.state.phase == Phase::InCall;
        let params = speaking.then(|| self.speech_params()).flatten();
        self.transcriber.sync(params);
    }

    fn speech_params(&self) -> Option<SpeechParams> {
        let (room, token) = (self.state.room_id.as_ref()?, self.seat.token.as_ref()?);
        let mut url = self.server.clone();
        // Only http and https servers are accepted at startup, so this can't fail
        let _ = url.set_scheme(if self.server.scheme() == "https" { "wss" } else { "ws" });
        url.set_path("/ws/speech");
        url.query_pairs_mut().clear().append_pair("room", room).append_pair("token", token);
        let mic = self.state.mic_id.as_ref().and_then(|id| self.state.mics.iter().find(|d| &d.id == id));
        Some(SpeechParams { url, lang: self.state.lang, mic: mic.map(|d| d.label.clone()) })
    }

    fn speech_notice(&mut self, msg: SpeechServerMessage) {
        let Some(notice) = self.transcriber.notice(msg) else { return };
        if let SpeechNotice::Paused { until_ms } = notice {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let wait = Duration::from_millis(until_ms.saturating_sub(now.as_millis() as u64));
            let tx = self.internal_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let _ = tx.send(Internal::SpeechResumed);
            });
        }
        self.state.speech_notice = Some(notice);
    }

    fn camera_wanted(&self) -> bool {
        matches!(self.state.phase, Phase::Setup | Phase::Starting | Phase::Waiting | Phase::InCall)
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
                chat,
                call_elapsed_ms,
            } => {
                (self.on_chat)(ChatEvent::Reset { my_id: client_id.clone(), messages: chat });
                self.seat.token = Some(token.clone());
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
            ServerMessage::Chat { message } => (self.on_chat)(ChatEvent::Added(message)),
            ServerMessage::ChatTranslation { id, translation } => {
                (self.on_chat)(ChatEvent::Translated { id, translation });
            }
            ServerMessage::Screenshot => {
                self.state.peer_screenshot = true;
                let tx = self.internal_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(PEER_SCREENSHOT_SHOWN).await;
                    let _ = tx.send(Internal::PeerScreenshotShown);
                });
            }
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
            force_relay: self.state.force_relay,
            polite: self.seat.polite,
            stream_id: self.stream_id.clone(),
        };
        match Peer::start(self.media.factory(), config, self.mic.clone(), self.camera.track(), self.peer_tx.clone()) {
            Ok(peer) => {
                self.peer = Some(peer);
                self.direct_retries = 0;
                self.next_direct_retry = None;
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
        self.transcriber.set_remote_audio(None);
        if let Some(pump) = self.remote_video.take() {
            pump.abort();
            self.sink.clear(Feed::Peer);
        }
        self.state.remote_video = false;
        self.state.stats = None;
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

    /// Tells the other side what we're sending, which the tracks alone can't say: a camera turned off still sends
    /// black frames.
    fn send_media(&self) {
        if self.peer.is_none() {
            return;
        }
        if let Some(signaling) = &self.signaling {
            signaling.send(ClientMessage::Media { cam_on: self.camera.sending(), screen: None });
        }
    }

    fn peer_event(&mut self, event: PeerEvent) {
        match event {
            PeerEvent::Signal(payload) => {
                if let Some(signaling) = &self.signaling {
                    signaling.send(ClientMessage::Signal { payload });
                }
            }
            PeerEvent::RemoteVideo { track } => {
                // Screen share (M4) will pick by stream id; until then every video is their camera
                if let Some(old) = self.remote_video.replace(video::pump(track, Feed::Peer, self.sink.clone())) {
                    old.abort();
                }
                self.state.remote_video = true;
            }
            PeerEvent::RemoteAudio { track } => self.transcriber.set_remote_audio(Some(track)),
            PeerEvent::Stats(stats) => {
                self.maybe_retry_direct(&stats);
                self.state.trying_direct = self.last_direct_retry.is_some_and(|at| at.elapsed() < TRYING_DIRECT_SHOWN);
                self.state.stats = Some(*stats);
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

    /// Upgrades a relayed call to direct P2P with an ICE restart, backing off between tries. Only the impolite side
    /// drives it, so the two sides don't restart over each other.
    fn maybe_retry_direct(&mut self, stats: &CallStats) {
        if stats.path == Path::Direct || self.state.force_relay || self.seat.polite {
            self.direct_retries = 0;
            self.next_direct_retry = None;
            return;
        }
        if stats.path != Path::Relay {
            return;
        }
        let now = Instant::now();
        let Some(next) = self.next_direct_retry else {
            self.next_direct_retry = Some(now + DIRECT_RETRY_DELAYS[0]);
            return;
        };
        if now < next {
            return;
        }
        self.direct_retries += 1;
        let delay =
            DIRECT_RETRY_DELAYS.get(self.direct_retries).or(DIRECT_RETRY_DELAYS.last()).copied().unwrap_or_default();
        self.next_direct_retry = Some(now + delay);
        self.last_direct_retry = Some(now);
        if let Some(peer) = &self.peer {
            peer.restart_ice();
        }
    }

    fn publish(&self) {
        (self.on_state)(self.state.clone());
    }
}
