//! `/ws/user`, a signed-in client's own socket: the friends list with presence, and call requests. Mirrors the
//! `/ws/user` part of `shared/protocol.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Presence {
    Online,
    InCall,
    Offline,
    /// From a newer server: shown as offline
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendState {
    pub username: String,
    pub presence: Presence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissedCall {
    pub from: String,
    /// Epoch ms
    pub at: u64,
}

/// A private room you left that the other person is still in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRoom {
    pub room_id: String,
    pub with: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallEndReason {
    Declined,
    Busy,
    Offline,
    Timeout,
    Cancelled,
    Unavailable,
    /// To the callee's other devices once one of them took the call
    AnsweredElsewhere,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum UserClientMessage {
    /// Rings every device of a friend; a new ring cancels the last
    Ring {
        to: String,
    },
    Cancel {
        call_id: String,
    },
    Answer {
        call_id: String,
        accept: bool,
    },
    /// Clears the missed calls in the next state
    MissedSeen,
    Ping,
}

/// Everything the friends screen shows; sent on connect and again whenever any of it changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendsLists {
    pub username: String,
    pub friends: Vec<FriendState>,
    /// Friend requests to you, and yours still waiting
    pub incoming: Vec<String>,
    pub outgoing: Vec<String>,
    pub blocked: Vec<String>,
    pub missed: Vec<MissedCall>,
    pub rooms: Vec<OpenRoom>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum UserServerMessage {
    State(FriendsLists),
    /// Your ring reached at least one of their devices
    Ringing {
        call_id: String,
        to: String,
    },
    Incoming {
        call_id: String,
        from: String,
    },
    /// To the caller and to the device that answered: join this room
    Answered {
        call_id: String,
        room_id: String,
    },
    CallEnded {
        call_id: String,
        reason: CallEndReason,
    },
    Pong,
    #[serde(other)]
    Unknown,
}
