//! Messages exchanged over `/ws`, `/ws/speech` and `/ws/user`, and the `/api` bodies, mirroring `shared/protocol.ts` by hand. The TypeScript file is
//! the source of truth: `tests/fixtures.rs` parses every example in `shared/protocol.fixtures.ts`, so a change
//! there that isn't followed here fails the tests.

mod api;
mod user;

pub use api::*;
pub use user::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// In UTF-16 code units, as the server counts it (`String.length`).
pub const MAX_CHAT_LENGTH: usize = 2000;
/// WebRTC stream ids are UUIDs (Firefox wraps them in braces); anything longer is not one.
pub const MAX_STREAM_ID_LENGTH: usize = 64;
/// `ROOM_ID_RE` in protocol.ts; [`is_room_id`] is what checks it.
pub const ROOM_ID_PATTERN: &str = "^[A-Za-z0-9_-]{6,32}$";
pub const TRANSCRIBE_SAMPLE_RATE: u32 = 16_000;
pub const MAX_UTTERANCE_S: u32 = 30;
/// Live text: what's recognized of an utterance while it's still being said, in UTF-16 code units.
pub const MAX_LIVE_LENGTH: usize = 500;
pub const MAX_LIVE_ID_LENGTH: usize = 64;

pub fn is_room_id(id: &str) -> bool {
    (6..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// What can be sent as a reaction: it flies over both sides' video and isn't stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reaction {
    #[serde(rename = "❤️")]
    Heart,
    #[serde(rename = "😂")]
    Laugh,
    #[serde(rename = "👍")]
    ThumbsUp,
    #[serde(rename = "😮")]
    Wow,
    #[serde(rename = "😢")]
    Sad,
    #[serde(rename = "🔥")]
    Fire,
    #[serde(rename = "🎉")]
    Party,
    #[serde(rename = "👏")]
    Clap,
}

impl Reaction {
    pub const ALL: [Reaction; 8] = [
        Reaction::Heart,
        Reaction::Laugh,
        Reaction::ThumbsUp,
        Reaction::Wow,
        Reaction::Sad,
        Reaction::Fire,
        Reaction::Party,
        Reaction::Clap,
    ];

    pub fn emoji(self) -> &'static str {
        match self {
            Reaction::Heart => "❤️",
            Reaction::Laugh => "😂",
            Reaction::ThumbsUp => "👍",
            Reaction::Wow => "😮",
            Reaction::Sad => "😢",
            Reaction::Fire => "🔥",
            Reaction::Party => "🎉",
            Reaction::Clap => "👏",
        }
    }
}

/// The two languages the chat is translated between, also what each side's speech is recognized in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Zh, Lang::En];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    /// ISO 639-1 code of the original message; may be neither of [`Lang`].
    pub lang: String,
    pub text: TranslatedText,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslatedText {
    pub zh: String,
    pub en: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    /// `clientId` of the sender.
    pub from: String,
    pub text: String,
    pub ts: u64,
    /// Transcribed from the sender's speech.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<bool>,
    /// The live text this line finishes, which it replaces in the subtitles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_id: Option<String>,
    /// Arrives after the message, in a `chat-translation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<Translation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDescription {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp: Option<String>,
}

/// Opaque WebRTC payloads relayed verbatim to the other peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SignalPayload {
    Description {
        description: SessionDescription,
    },
    /// Browsers send `RTCIceCandidateInit` (or null at the end), so it's kept as JSON.
    Candidate {
        candidate: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum ClientMessage {
    Signal {
        payload: SignalPayload,
    },
    Chat {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spoken: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_id: Option<String>,
    },
    /// The utterance being said so far, in the speaker's language. Each one replaces the last with the same id; the
    /// spoken chat line with that `live_id` finishes it, and empty text drops it.
    ChatLive {
        id: String,
        text: String,
        lang: Lang,
    },
    Leave,
    Ping,
    /// Tells the other person you saved a picture of the call.
    Screenshot,
    Reaction {
        emoji: Reaction,
    },
    /// What you're sending, which the tracks alone can't say. Sent on every change and whenever the peer
    /// connection is (re)made. `screen` is the id of the stream the shared screen arrives in.
    Media {
        cam_on: bool,
        screen: Option<String>,
        /// You muted the other person's sound. Missing from older builds, which means false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        speaker_off: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum ServerMessage {
    Joined {
        client_id: String,
        /// Secret for resuming this seat and for `/ws/speech`. Never log it.
        token: String,
        /// This socket replaced the same client's dropped socket.
        resumed: bool,
        peer_present: bool,
        polite: bool,
        ice_servers: Vec<IceServerConfig>,
        chat: Vec<ChatMessage>,
        /// How long the call has run, when the other member is here. A duration: clocks differ.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_elapsed_ms: Option<u64>,
    },
    PeerJoined {
        polite: bool,
        call_elapsed_ms: u64,
    },
    PeerLeft,
    /// The room can't be joined; stop reconnecting. No reason: two people are already in it.
    Full {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<FullReason>,
    },
    Signal {
        payload: SignalPayload,
    },
    Chat {
        message: ChatMessage,
    },
    ChatTranslation {
        id: String,
        translation: Translation,
    },
    /// The other person's utterance so far; see [`ClientMessage::ChatLive`].
    ChatLive {
        id: String,
        text: String,
        lang: Lang,
    },
    Pong,
    Screenshot,
    Reaction {
        emoji: Reaction,
    },
    Media {
        cam_on: bool,
        screen: Option<String>,
        /// You muted the other person's sound. Missing from older builds, which means false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        speaker_off: bool,
    },
    Error {
        message: String,
    },
    /// A type this build doesn't know, from a newer server: ignored rather than dropping the connection.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FullReason {
    /// An anonymous room's call ran out its time.
    TimeLimit,
    /// A private room of two other accounts.
    NotInvited,
    /// The same account joined it from another device.
    Elsewhere,
    /// A reason this build doesn't know, from a newer server.
    #[serde(other)]
    Unknown,
}

/// `/ws/speech` carries one utterance at a time: `Start`, its audio as binary frames of 16-bit little-endian mono
/// PCM at [`TRANSCRIBE_SAMPLE_RATE`] while it's said, then `End` to transcribe it or `Cancel` to drop it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SpeechClientMessage {
    Start { lang: Lang },
    End,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SpeechServerMessage {
    /// Every speech model is out of quota until this epoch ms.
    Paused { until: u64 },
    /// The server has no speech model configured.
    Unavailable,
    #[serde(other)]
    Unknown,
}
