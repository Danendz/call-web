//! Owns the call's state and everything that changes it, on its own tokio runtime. The UI only sends
//! [`Command`]s and renders each [`State`] it's handed.

use std::thread;

use protocol::{Lang, ServerMessage};
use tokio::sync::mpsc;
use url::Url;

use crate::room;
use crate::signaling::{SignalEvent, Signaling, SocketStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No room picked yet: new room or join by link
    Start,
    /// Room picked, not joined: the pre-join screen
    Setup,
    Starting,
    /// Joined; `peer_present` says whether the other person is here
    Waiting,
    Full,
    Ended,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub phase: Phase,
    pub lang: Lang,
    pub room_id: Option<String>,
    /// The web address of the room, for sharing: it opens in a browser or in this app.
    pub room_url: Option<String>,
    pub peer_present: bool,
    /// The socket dropped and is being reopened; the seat is kept for a while meanwhile.
    pub reconnecting: bool,
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
}

/// Handle to the engine thread. Dropping it leaves the room and stops the thread.
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
}

impl Engine {
    /// `on_state` runs on the engine's thread for every change; the UI hops to its own thread from there.
    pub fn start(server: Url, lang: Lang, on_state: impl Fn(State) + Send + 'static) -> std::io::Result<Self> {
        let (commands, rx) = mpsc::unbounded_channel();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        thread::Builder::new().name("engine".into()).spawn(move || {
            runtime.block_on(Controller::new(server, lang, Box::new(on_state)).run(rx));
        })?;
        Ok(Self { commands })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

struct Controller {
    server: Url,
    state: State,
    on_state: Box<dyn Fn(State) + Send>,
    signaling: Option<Signaling>,
    events_tx: mpsc::UnboundedSender<SignalEvent>,
    events: mpsc::UnboundedReceiver<SignalEvent>,
}

impl Controller {
    fn new(server: Url, lang: Lang, on_state: Box<dyn Fn(State) + Send>) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let state = State {
            phase: Phase::Start,
            lang,
            room_id: None,
            room_url: None,
            peer_present: false,
            reconnecting: false,
        };
        Self { server, state, on_state, signaling: None, events_tx, events }
    }

    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.publish();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.command(command),
                    None => break,
                },
                Some(event) = self.events.recv() => self.event(event),
            }
        }
        if let Some(signaling) = self.signaling.take() {
            signaling.leave();
            // Give the leave a moment to go out before the runtime shuts down
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
        }
        self.publish();
    }

    fn open_room(&mut self, id: String) {
        self.disconnect();
        self.state.room_url = Some(room::room_url(&self.server, &id));
        self.state.room_id = Some(id);
        self.state.phase = Phase::Setup;
    }

    fn join(&mut self) {
        let Some(room_id) = self.state.room_id.clone() else { return };
        self.disconnect();
        self.signaling = Some(Signaling::connect(&self.server, &room_id, self.events_tx.clone()));
        self.state.phase = Phase::Starting;
    }

    fn disconnect(&mut self) {
        if let Some(signaling) = self.signaling.take() {
            signaling.leave();
        }
        // Events from the old socket still queued must not touch the next call
        while self.events.try_recv().is_ok() {}
        self.state.peer_present = false;
        self.state.reconnecting = false;
    }

    fn event(&mut self, event: SignalEvent) {
        match event {
            SignalEvent::Status(status) => self.state.reconnecting = status == SocketStatus::Reconnecting,
            SignalEvent::Message(msg) => self.server_message(msg),
        }
        self.publish();
    }

    fn server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Joined { client_id, token, peer_present, .. } => {
                if let Some(signaling) = &self.signaling {
                    signaling.set_identity(client_id, token);
                }
                self.state.peer_present = peer_present;
                self.state.phase = Phase::Waiting;
            }
            ServerMessage::PeerJoined { .. } => self.state.peer_present = true,
            ServerMessage::PeerLeft => self.state.peer_present = false,
            ServerMessage::Full => {
                self.signaling = None;
                self.state.phase = Phase::Full;
            }
            ServerMessage::Error { message } => tracing::warn!("server error: {message}"),
            // Media, chat and the rest arrive with the milestones that use them
            _ => {}
        }
    }

    fn publish(&self) {
        (self.on_state)(self.state.clone());
    }
}
