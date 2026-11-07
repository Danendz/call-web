//! Owns the call's state and everything that changes it, on its own tokio runtime. The UI only sends
//! [`Command`]s and renders each [`State`] it's handed. Mirrors the web's `controller.ts`.

mod api;
mod call;
mod devices;

pub use api::{ChatEvent, Command, Engine, Link, Phase, State};

use std::sync::Arc;
use std::time::{Duration, Instant};

use libwebrtc::media_stream_track::MediaStreamTrack;
use protocol::{ClientMessage, IceServerConfig, Lang, MAX_CHAT_LENGTH, SpeechServerMessage};
use tokio::sync::mpsc;
use url::Url;

use crate::camera::CameraAccess;
use crate::camera::local::LocalCamera;
use crate::level::{Level, Meter};
use crate::media::Media;
use crate::peer::{Peer, PeerEvent};
use crate::remote::RemoteVideos;
use crate::room;
use crate::screen::{self, LocalScreen};
use crate::signaling::{SignalEvent, Signaling, SocketStatus};
use crate::speech::{SpeechNotice, Transcriber};
use crate::video::VideoSink;

/// What the engine tells itself from other threads and timers.
enum Internal {
    CameraAccess(CameraAccess),
    PeerScreenshotShown,
    /// Speech to chat's quota is back
    SpeechResumed,
    /// The share stopped by itself: picker cancelled, window closed, the OS's "stop sharing"
    ScreenEnded,
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
    remote: RemoteVideos,
    screen: LocalScreen,
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
        let screen = LocalScreen::new(media.factory());
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
            remote_screen: false,
            sharing: false,
            screen_picker: None,
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
            remote: RemoteVideos::new(sink),
            screen,
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
                    Internal::ScreenEnded => self.stop_share(),
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
            Command::ToggleShare if self.screen.sharing() => self.stop_share(),
            // Only in a call, like the web's menu item
            Command::ToggleShare if self.peer.is_some() => {
                if screen::SYSTEM_PICKER {
                    self.start_share(None);
                } else {
                    self.state.screen_picker = Some(screen::sources());
                }
            }
            Command::ToggleShare => {}
            Command::ShareSource(id) => {
                if self.peer.is_some() && !self.screen.sharing() {
                    self.start_share(Some(id));
                }
            }
            Command::CloseScreenPicker => self.state.screen_picker = None,
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
        self.stop_share();
        self.remote.reset();
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

    fn publish(&self) {
        (self.on_state)(self.state.clone());
    }
}
