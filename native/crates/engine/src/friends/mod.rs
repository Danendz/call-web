//! The account and friends: signing in, the friends list with presence, and call requests both ways. Runs apart
//! from the call [`crate::Engine`] on its own thread, like the web's `FriendsSocket` next to `CallController`; an
//! answered call comes out as [`FriendsEvent::Answered`] for the app to join.

mod calls;
mod http;
mod socket;
mod state;

use std::thread;
use std::time::Duration;

use protocol::{ApiErrorCode, InvitesResponse, MeResponse, NewInviteResponse, SessionResponse, UserClientMessage};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use url::Url;

use http::Api;
use socket::{SocketEvent, UserSocket};
pub use state::{
    Account, FriendAction, FriendsCommand, FriendsEvent, FriendsState, IncomingCall, Outgoing, SessionToken,
};

/// Retry after this when the server couldn't be reached to check the session.
const RECHECK_AFTER: Duration = Duration::from_secs(10);

/// What finished on a blocking thread.
enum Done {
    /// The saved session checked out (Some), was refused (None), or the server couldn't be reached (Err)
    Checked(Result<Option<String>, ApiErrorCode>),
    Session(Result<SessionResponse, ApiErrorCode>),
    /// An account change: Ok carries the follow-up for the state
    Change(Result<Change, ApiErrorCode>),
}

enum Change {
    Nothing,
    Invites(InvitesResponse),
    Invite(NewInviteResponse),
    Password,
    Deleted,
}

/// Handle to the friends thread. Dropping it closes `/ws/user` and stops the thread.
pub struct Friends {
    commands: mpsc::UnboundedSender<FriendsCommand>,
}

impl Friends {
    /// `session`: the token kept from last time. `on_state` and `on_event` run on the friends thread.
    pub fn start(
        server: Url,
        session: Option<SessionToken>,
        on_state: impl Fn(FriendsState) + Send + 'static,
        on_event: impl Fn(FriendsEvent) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, rx) = mpsc::unbounded_channel();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        thread::Builder::new().name("friends".into()).spawn(move || {
            runtime.block_on(Controller::new(server, session, Box::new(on_state), Box::new(on_event)).run(rx));
        })?;
        Ok(Self { commands })
    }

    pub fn send(&self, command: FriendsCommand) {
        let _ = self.commands.send(command);
    }
}

struct Controller {
    api: std::sync::Arc<Api>,
    server: Url,
    session: Option<SessionToken>,
    state: FriendsState,
    on_state: Box<dyn Fn(FriendsState) + Send>,
    on_event: Box<dyn Fn(FriendsEvent) + Send>,
    socket: Option<UserSocket>,
    socket_tx: mpsc::UnboundedSender<SocketEvent>,
    socket_rx: mpsc::UnboundedReceiver<SocketEvent>,
    done_tx: mpsc::UnboundedSender<Done>,
    done_rx: mpsc::UnboundedReceiver<Done>,
}

impl Controller {
    fn new(
        server: Url,
        session: Option<SessionToken>,
        on_state: Box<dyn Fn(FriendsState) + Send>,
        on_event: Box<dyn Fn(FriendsEvent) + Send>,
    ) -> Self {
        let (socket_tx, socket_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        Self {
            api: std::sync::Arc::new(Api::new(server.clone())),
            server,
            session,
            state: FriendsState::new(Account::Loading),
            on_state,
            on_event,
            socket: None,
            socket_tx,
            socket_rx,
            done_tx,
            done_rx,
        }
    }

    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<FriendsCommand>) {
        self.check_session();
        self.publish();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.command(command),
                    None => break,
                },
                Some(event) = self.socket_rx.recv() => self.socket_event(event),
                Some(done) = self.done_rx.recv() => self.done(done),
            }
            self.publish();
        }
    }

    /// Runs `f` with the API on a blocking thread; its result comes back as a [`Done`].
    fn spawn(&self, f: impl FnOnce(&Api, Option<&SessionToken>) -> Done + Send + 'static) {
        let (api, session, tx) = (self.api.clone(), self.session.clone(), self.done_tx.clone());
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(f(&api, session.as_ref()));
        });
    }

    fn check_session(&self) {
        self.spawn(|api, session| Done::Checked(whose(api, session)));
    }

    fn command(&mut self, command: FriendsCommand) {
        match command {
            FriendsCommand::SignIn { username, password } => {
                self.start_session("/api/auth/login", json!({ "username": username, "password": password }));
            }
            FriendsCommand::Register { username, password, invite } => {
                self.start_session(
                    "/api/auth/register",
                    json!({ "username": username, "password": password, "invite": invite }),
                );
            }
            FriendsCommand::Recover { username, recovery_code, password } => {
                let body = json!({ "username": username, "recoveryCode": recovery_code, "password": password });
                self.start_session("/api/auth/recover", body);
            }
            FriendsCommand::SignOut => {
                self.spawn(|api, session| {
                    let _ = api.call("/api/auth/logout", session, Some(json!({})));
                    Done::Change(Ok(Change::Nothing))
                });
                self.signed_out();
            }
            FriendsCommand::RecoverySaved => self.state.recovery_code = None,
            FriendsCommand::ClearError => {
                self.state.error = None;
                self.state.password_changed = false;
                self.state.invite = None;
            }
            FriendsCommand::Friend(action, username) => {
                let name = username.trim().to_lowercase();
                if !protocol::is_username(&name) {
                    self.state.error = Some(ApiErrorCode::InvalidUsername);
                    return;
                }
                // The new lists arrive over /ws/user
                self.change(move |api, session| {
                    api.call(action.path(), session, Some(json!({ "username": name }))).map(|_| Change::Nothing)
                });
            }
            FriendsCommand::Ring(to) => {
                if let Some(socket) = &self.socket {
                    socket.send(UserClientMessage::Ring { to: to.clone() });
                    self.state.outgoing = Some(Outgoing { to, call_id: None });
                    self.state.notice = None;
                }
            }
            FriendsCommand::CancelRing => {
                if let (Some(socket), Some(call_id)) =
                    (&self.socket, self.state.outgoing.as_ref().and_then(|o| o.call_id.clone()))
                {
                    socket.send(UserClientMessage::Cancel { call_id });
                }
                self.state.outgoing = None;
            }
            FriendsCommand::Answer { call_id, accept } => {
                if let Some(socket) = &self.socket {
                    socket.send(UserClientMessage::Answer { call_id: call_id.clone(), accept });
                }
                self.state.incoming.retain(|c| c.call_id != call_id);
            }
            FriendsCommand::MissedSeen => {
                if let Some(socket) = &self.socket {
                    socket.send(UserClientMessage::MissedSeen);
                }
            }
            FriendsCommand::DismissNotice => self.state.notice = None,
            FriendsCommand::LoadInvites => {
                self.change(|api, session| api.call_as("/api/invites", session, None).map(Change::Invites));
            }
            FriendsCommand::NewInvite => {
                self.change(|api, session| api.call_as("/api/invites", session, Some(json!({}))).map(Change::Invite));
            }
            FriendsCommand::ChangePassword { current, new } => {
                self.change(move |api, session| {
                    api.call("/api/auth/password", session, Some(json!({ "password": current, "newPassword": new })))
                        .map(|_| Change::Password)
                });
            }
            FriendsCommand::DeleteAccount { password } => {
                self.change(move |api, session| {
                    api.call("/api/auth/delete", session, Some(json!({ "password": password })))
                        .map(|_| Change::Deleted)
                });
            }
        }
    }

    fn start_session(&mut self, path: &'static str, mut body: Value) {
        if let Value::Object(fields) = &mut body {
            fields.insert("client".into(), json!("native"));
        }
        self.state.busy = true;
        self.state.error = None;
        self.spawn(move |api, _| Done::Session(api.call_as(path, None, Some(body))));
    }

    fn change(&mut self, f: impl FnOnce(&Api, Option<&SessionToken>) -> Result<Change, ApiErrorCode> + Send + 'static) {
        self.state.busy = true;
        self.state.error = None;
        self.state.password_changed = false;
        self.spawn(move |api, session| Done::Change(f(api, session)));
    }

    fn done(&mut self, done: Done) {
        match done {
            Done::Checked(Ok(Some(username))) => self.signed_in(username),
            Done::Checked(Ok(None)) => self.signed_out(),
            Done::Checked(Err(ApiErrorCode::Unavailable)) if self.session.is_none() => {
                // 503 too: the server has no accounts. Either way there's nothing to sign in to right now
                self.state.account = Account::SignedOut { accounts_on: false };
            }
            Done::Checked(Err(_)) => {
                // Can't reach the server: keep the session and ask again in a while
                let tx = self.done_tx.clone();
                let (api, session) = (self.api.clone(), self.session.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(RECHECK_AFTER).await;
                    let _ = tokio::task::spawn_blocking(move || tx.send(Done::Checked(whose(&api, session.as_ref()))))
                        .await;
                });
            }
            Done::Session(result) => {
                self.state.busy = false;
                match result {
                    Ok(SessionResponse { username, token: Some(token), recovery_code }) => {
                        self.session = Some(SessionToken(token));
                        (self.on_event)(FriendsEvent::Session(self.session.clone()));
                        self.state.recovery_code = recovery_code;
                        self.signed_in(username);
                    }
                    Ok(_) => self.state.error = Some(ApiErrorCode::Unavailable),
                    Err(e) => self.state.error = Some(e),
                }
            }
            Done::Change(result) => {
                self.state.busy = false;
                match result {
                    Ok(Change::Nothing) => {}
                    Ok(Change::Invites(i)) => self.state.invites = Some((i.unused, i.max)),
                    Ok(Change::Invite(invite)) => {
                        self.state.invite = Some(invite.code);
                        self.command(FriendsCommand::LoadInvites);
                    }
                    Ok(Change::Password) => self.state.password_changed = true,
                    Ok(Change::Deleted) => self.signed_out(),
                    Err(ApiErrorCode::Unauthorized) if self.session.is_some() => self.signed_out(),
                    Err(e) => self.state.error = Some(e),
                }
            }
        }
    }

    fn signed_in(&mut self, username: String) {
        self.state.account = Account::SignedIn { username };
        if self.socket.is_none()
            && let Some(session) = self.session.clone()
        {
            // Events from an earlier socket must not touch this one
            while self.socket_rx.try_recv().is_ok() {}
            self.socket = Some(UserSocket::connect(&self.server, session, self.socket_tx.clone()));
        }
    }

    fn signed_out(&mut self) {
        self.socket = None;
        if self.session.take().is_some() {
            (self.on_event)(FriendsEvent::Session(None));
        }
        let error = self.state.error;
        self.state = FriendsState { error, ..FriendsState::new(Account::SignedOut { accounts_on: true }) };
    }

    fn publish(&self) {
        (self.on_state)(self.state.clone());
    }
}

/// Whose the session is: Some(username), None when there's no session or the server refused it.
fn whose(api: &Api, session: Option<&SessionToken>) -> Result<Option<String>, ApiErrorCode> {
    match api.call_as::<MeResponse>("/api/me", session, None) {
        Ok(me) if session.is_some() => Ok(Some(me.username)),
        Ok(_) | Err(ApiErrorCode::Unauthorized) => Ok(None),
        Err(e) => Err(e),
    }
}
