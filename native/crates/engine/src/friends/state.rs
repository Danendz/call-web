//! What the friends thread shows and takes: its state, commands and events.

use protocol::{ApiErrorCode, CallEndReason, FriendsLists};

/// The account's session token. Its `Debug` doesn't show it, so it can't end up in a log.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(pub String);

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(…)")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    /// Checking the saved session with the server
    Loading,
    /// `accounts_on`: false when the server has no accounts (no database), so there's nothing to sign in to
    SignedOut {
        accounts_on: bool,
    },
    SignedIn {
        username: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub to: String,
    /// None until the server says it's ringing
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCall {
    pub call_id: String,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FriendsState {
    pub account: Account,
    /// A sign-in or account change is on its way to the server
    pub busy: bool,
    /// Why the last sign-in or account change was refused, until the next one
    pub error: Option<ApiErrorCode>,
    /// New after register or recover: shown once, until [`FriendsCommand::RecoverySaved`]
    pub recovery_code: Option<String>,
    /// `/ws/user` is open
    pub connected: bool,
    pub lists: Option<FriendsLists>,
    pub outgoing: Option<Outgoing>,
    pub incoming: Vec<IncomingCall>,
    /// How your last ring ended, and with whom, until dismissed
    pub notice: Option<(CallEndReason, String)>,
    /// A new invite code: shown once
    pub invite: Option<String>,
    /// Unused invite codes, and the most one account can hold
    pub invites: Option<(u32, u32)>,
    pub password_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendAction {
    Request,
    Accept,
    /// Unfriends, declines their request or withdraws yours
    Remove,
    Block,
    Unblock,
}

impl FriendAction {
    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Request => "/api/friends/request",
            Self::Accept => "/api/friends/accept",
            Self::Remove => "/api/friends/remove",
            Self::Block => "/api/friends/block",
            Self::Unblock => "/api/friends/unblock",
        }
    }
}

#[derive(Debug, Clone)]
pub enum FriendsCommand {
    SignIn {
        username: String,
        password: String,
    },
    Register {
        username: String,
        password: String,
        invite: String,
    },
    Recover {
        username: String,
        recovery_code: String,
        password: String,
    },
    SignOut,
    RecoverySaved,
    ClearError,
    Friend(FriendAction, String),
    Ring(String),
    CancelRing,
    Answer {
        call_id: String,
        accept: bool,
    },
    MissedSeen,
    DismissNotice,
    /// Counts the unused invite codes, for the account screen
    LoadInvites,
    NewInvite,
    ChangePassword {
        current: String,
        new: String,
    },
    DeleteAccount {
        password: String,
    },
}

#[derive(Debug, Clone)]
pub enum FriendsEvent {
    /// Keep this token for next time (None: forget it), and use it on room sockets
    Session(Option<SessionToken>),
    /// A call you made or took was answered: join this room
    Answered(String),
}

impl FriendsState {
    pub(super) fn new(account: Account) -> Self {
        Self {
            account,
            busy: false,
            error: None,
            recovery_code: None,
            connected: false,
            lists: None,
            outgoing: None,
            incoming: Vec::new(),
            notice: None,
            invite: None,
            invites: None,
            password_changed: false,
        }
    }
}
