//! What the UI sees of the engine: the handle it sends [`Command`]s through, and the [`State`] and [`ChatEvent`]s it
//! gets back.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use protocol::{ChatMessage, FullReason, Lang, Reaction, Translation};
use tokio::sync::mpsc;
use url::Url;

use super::Controller;
use crate::camera::CameraAccess;
use crate::level::Level;
use crate::media::Device;
use crate::speech::{Model, ModelStatus, SpeechNotice};
use crate::stats::CallStats;
use crate::video::VideoSink;

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
    /// A mic can be used, even where there's no list to pick from (Android)
    pub has_mic: bool,
    pub mics: Vec<Device>,
    pub mic_id: Option<String>,
    /// Where the call plays; empty on Android, which has its audio routes instead
    pub speakers: Vec<Device>,
    pub speaker_id: Option<String>,
    /// We muted the other person, for this call only: every call starts with sound
    pub speaker_off: bool,
    /// They muted us (from their `media` messages)
    pub remote_speaker_off: bool,
    /// The other person's camera is on (from their `media` messages).
    pub remote_cam_on: bool,
    /// Their camera's track has arrived; frames go to the sink as [`crate::Feed::Peer`]
    pub remote_video: bool,
    /// They're sharing their screen and it's arriving; frames go to the sink as [`crate::Feed::Screen`]
    pub remote_screen: bool,
    /// We're sharing our screen
    pub sharing: bool,
    /// Screens and windows to pick from, while the app's own picker is open (where the OS has none)
    pub screen_picker: Option<Vec<Device>>,
    pub cam_on: bool,
    pub cam_access: CameraAccess,
    pub cams: Vec<Device>,
    pub cam_id: Option<String>,
    /// We send a 9:16 picture instead of 16:9
    pub portrait: bool,
    /// Why the room can't be joined, in [`Phase::Full`]; None: two people are already in it
    pub full_reason: Option<FullReason>,
    pub stats: Option<CallStats>,
    /// The other person just saved a picture of the call
    pub peer_screenshot: bool,
    /// Speech to chat is on: what you say is posted to the chat while someone's there to read it
    pub transcribe_on: bool,
    pub speech_notice: Option<SpeechNotice>,
    /// The on-device speech model, where there's a data folder to keep it in
    pub model: ModelStatus,
    /// Speech goes to the on-device model once it's ready, instead of the server
    pub on_device: bool,
    /// The live model, where there's a data folder to keep it in
    pub live: ModelStatus,
    /// Live subtitles are on: once the live model is there, it recognizes your speech and sends it as you speak
    pub live_subtitles: bool,
    /// The translation model for live subtitles in `lang`, where there's a data folder to keep it in
    pub translation: ModelStatus,
    /// Everything goes through our TURN server (a stats overlay switch, for testing the relay)
    pub force_relay: bool,
    /// An ICE restart to get off the relay just went out
    pub trying_direct: bool,
}

#[derive(Debug, Clone)]
pub enum Command {
    NewRoom,
    /// A room id, or a link someone pasted, already checked with [`crate::room::parse_room`].
    OpenRoom(String),
    SetLang(Lang),
    /// The signed-in account's token for the room socket (a private room needs it), or None once signed out
    SetSession(Option<crate::friends::SessionToken>),
    Join,
    HangUp,
    /// Back to the start screen
    Home,
    ToggleMic,
    /// Any time: in a call capture restarts on the new mic, which the same track then sends
    SelectMic(String),
    /// Any time: in a call playout restarts on the new speaker
    SelectSpeaker(String),
    /// Mutes the other person here, and tells them
    ToggleSpeaker,
    ToggleCam,
    /// Any time: the new camera feeds the same track, so a call needs no renegotiation
    SelectCam(String),
    SetPortrait(bool),
    SetForceRelay(bool),
    /// DeepFilterNet on the mic instead of WebRTC's own noise suppression; any time, live in a call
    SetNoiseSuppression(bool),
    /// Any time: a call renegotiates onto it
    SetVideoCodec(crate::VideoCodec),
    /// Sent as typed, trimmed; nothing if it's empty or longer than the server takes
    SendChat(String),
    /// Tells the other person a picture of the call was saved
    NotifyScreenshot,
    /// Flies over both sides' video; only in a call
    SendReaction(Reaction),
    SetTranscribe(bool),
    /// Downloads an on-device speech model, picking up where an earlier try stopped
    DownloadModel(Model),
    CancelModelDownload(Model),
    DeleteModel(Model),
    SetOnDevice(bool),
    SetLiveSubtitles(bool),
    /// Downloads the translation model `lang` needs, picking up where an earlier try stopped
    DownloadTranslation,
    DeleteTranslation,
    /// Starts sharing (with the OS's picker, or opening the app's) or stops
    ToggleShare,
    /// A screen or window from the app's picker
    ShareSource(String),
    CloseScreenPicker,
}

/// The chat, as changes: a whole new list on joining, then single messages and translations, so the UI updates its
/// list in place. Kept out of [`State`], which would otherwise copy every message on every change.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    Reset {
        my_id: String,
        messages: Vec<ChatMessage>,
    },
    Added(ChatMessage),
    Translated {
        id: String,
        translation: Translation,
    },
    /// What the other person is saying right now; empty text drops it. The spoken message with this `live_id`
    /// finishes it.
    Live {
        id: String,
        text: String,
    },
    /// A reaction to fly over the video: one we sent, or one from the other person. Never stored.
    Reaction {
        emoji: Reaction,
        mine: bool,
    },
}

/// Handle to the engine thread. Dropping it leaves the room and stops the thread.
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    level: Level,
}

impl Engine {
    /// `on_state` runs on the engine's thread for every change; the UI hops to its own thread from there.
    /// Video frames go to `sink`, from video threads; see [`VideoSink`].
    /// `data_dir` is where the on-device speech model is kept; `on_device` whether it's used once it's there.
    pub fn start(
        server: Url,
        lang: Lang,
        data_dir: Option<PathBuf>,
        on_device: bool,
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
                Controller::new(
                    server,
                    lang,
                    data_dir,
                    on_device,
                    controller_level,
                    sink,
                    Box::new(on_state),
                    Box::new(on_chat),
                )
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
