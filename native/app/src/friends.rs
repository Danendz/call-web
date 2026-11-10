//! The account and friends: the friends thread's state into the window, the window's callbacks into commands, and
//! the session token kept between launches.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, Local};
use engine::friends::{Account, FriendAction, Friends, FriendsCommand, FriendsEvent, FriendsState, SessionToken};
use engine::{ApiErrorCode, CallEndReason, Command, Engine, Presence};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::settings;
use crate::ui::{AppWindow, FriendRow, MissedRow, RoomRow};

thread_local! {
    // What the friends thread's state and events reach on the UI thread: models and the engine aren't `Send`
    #[cfg_attr(target_os = "android", allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android"))]
    static LINKS: RefCell<Option<(Models, Rc<Engine>)>> = const { RefCell::new(None) };
}

/// Starts the friends thread with the saved session, and connects it to the window and the call engine.
pub fn start(ui: &AppWindow, engine: &Rc<Engine>, server: url::Url) -> anyhow::Result<Rc<Friends>> {
    let session = load_session();
    // A private room needs the session on the room socket
    engine.send(Command::SetSession(session.clone()));
    LINKS.with(|links| *links.borrow_mut() = Some((Models::new(ui), engine.clone())));

    let weak = ui.as_weak();
    let friends = Rc::new(Friends::start(
        server,
        session,
        move |state| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                LINKS.with(|links| {
                    if let Some((models, _)) = links.borrow().as_ref() {
                        render(&ui, models, &state);
                    }
                });
            });
        },
        move |event| {
            let _ = slint::invoke_from_event_loop(move || event_on_ui(event));
        },
    )?);
    wire(ui, &friends);
    Ok(friends)
}

fn event_on_ui(event: FriendsEvent) {
    LINKS.with(|links| {
        let Some(engine) = links.borrow().as_ref().map(|(_, engine)| engine.clone()) else { return };
        match event {
            FriendsEvent::Session(session) => {
                save_session(session.as_ref());
                engine.send(Command::SetSession(session));
            }
            FriendsEvent::Answered(room_id) => {
                // The click on Accept, or the other person's, was the go-ahead: straight into the call
                engine.send(Command::OpenRoom(room_id));
                engine.send(Command::Join);
            }
        }
    });
}

fn wire(ui: &AppWindow, friends: &Rc<Friends>) {
    let send = |make: fn() -> FriendsCommand| {
        let friends = friends.clone();
        move || friends.send(make())
    };
    ui.on_sign_out(send(|| FriendsCommand::SignOut));
    ui.on_recovery_saved(send(|| FriendsCommand::RecoverySaved));
    ui.on_clear_account_error(send(|| FriendsCommand::ClearError));
    ui.on_cancel_ring(send(|| FriendsCommand::CancelRing));
    ui.on_missed_seen(send(|| FriendsCommand::MissedSeen));
    ui.on_dismiss_call_notice(send(|| FriendsCommand::DismissNotice));
    ui.on_load_invites(send(|| FriendsCommand::LoadInvites));
    ui.on_new_invite(send(|| FriendsCommand::NewInvite));
    ui.on_sign_in({
        let friends = friends.clone();
        move |username, password| {
            friends.send(FriendsCommand::SignIn { username: username.into(), password: password.into() })
        }
    });
    ui.on_register({
        let friends = friends.clone();
        move |username, password, invite| {
            friends.send(FriendsCommand::Register {
                username: username.into(),
                password: password.into(),
                invite: invite.into(),
            });
        }
    });
    ui.on_recover({
        let friends = friends.clone();
        move |username, recovery_code, password| {
            friends.send(FriendsCommand::Recover {
                username: username.into(),
                recovery_code: recovery_code.into(),
                password: password.into(),
            });
        }
    });
    ui.on_friend_action({
        let friends = friends.clone();
        move |action, username| {
            let action = match action.as_str() {
                "request" => FriendAction::Request,
                "accept" => FriendAction::Accept,
                "remove" => FriendAction::Remove,
                "block" => FriendAction::Block,
                "unblock" => FriendAction::Unblock,
                _ => return,
            };
            friends.send(FriendsCommand::Friend(action, username.into()));
        }
    });
    ui.on_ring({
        let friends = friends.clone();
        move |username| friends.send(FriendsCommand::Ring(username.into()))
    });
    ui.on_answer({
        let friends = friends.clone();
        let weak = ui.as_weak();
        move |accept| {
            let Some(ui) = weak.upgrade() else { return };
            // The call shown is the oldest one ringing
            let call_id = ui.get_incoming_call_id().to_string();
            if !call_id.is_empty() {
                friends.send(FriendsCommand::Answer { call_id, accept });
            }
        }
    });
    ui.on_change_password({
        let friends = friends.clone();
        move |current, new| friends.send(FriendsCommand::ChangePassword { current: current.into(), new: new.into() })
    });
    ui.on_delete_account({
        let friends = friends.clone();
        move |password| friends.send(FriendsCommand::DeleteAccount { password: password.into() })
    });
}

/// The window's lists, updated in place.
struct Models {
    friends: ModelRc<FriendRow>,
    requests: ModelRc<SharedString>,
    pending: ModelRc<SharedString>,
    blocked: ModelRc<SharedString>,
    missed: ModelRc<MissedRow>,
    rooms: ModelRc<RoomRow>,
}

impl Models {
    fn new(ui: &AppWindow) -> Self {
        let models = Self {
            friends: ModelRc::from(Rc::new(VecModel::default())),
            requests: ModelRc::from(Rc::new(VecModel::default())),
            pending: ModelRc::from(Rc::new(VecModel::default())),
            blocked: ModelRc::from(Rc::new(VecModel::default())),
            missed: ModelRc::from(Rc::new(VecModel::default())),
            rooms: ModelRc::from(Rc::new(VecModel::default())),
        };
        ui.set_friends(models.friends.clone());
        ui.set_friend_requests(models.requests.clone());
        ui.set_pending_requests(models.pending.clone());
        ui.set_blocked(models.blocked.clone());
        ui.set_missed(models.missed.clone());
        ui.set_open_rooms(models.rooms.clone());
        models
    }
}

fn render(ui: &AppWindow, models: &Models, state: &FriendsState) {
    let (account, username) = match &state.account {
        Account::Loading => ("loading", ""),
        Account::SignedOut { accounts_on: true } => ("signed-out", ""),
        Account::SignedOut { accounts_on: false } => ("off", ""),
        Account::SignedIn { username } => ("signed-in", username.as_str()),
    };
    ui.set_account(account.into());
    ui.set_username(username.into());
    if account == "signed-in" {
        ui.set_sign_in_open(false);
    }
    ui.set_account_busy(state.busy);
    ui.set_account_error(state.error.map(error_code).unwrap_or_default().into());
    ui.set_recovery_code(state.recovery_code.clone().unwrap_or_default().into());
    ui.set_invite(state.invite.clone().unwrap_or_default().into());
    let (unused, max) = state.invites.unwrap_or_default();
    ui.set_invites_unused(i32::try_from(unused).unwrap_or(i32::MAX));
    ui.set_invites_max(i32::try_from(max).unwrap_or(i32::MAX));
    ui.set_password_changed(state.password_changed);
    ui.set_friends_connected(state.connected);
    ui.set_calling(state.outgoing.as_ref().map(|o| o.to.as_str()).unwrap_or_default().into());
    let incoming = state.incoming.first();
    ui.set_incoming_from(incoming.map(|c| c.from.as_str()).unwrap_or_default().into());
    ui.set_incoming_call_id(incoming.map(|c| c.call_id.as_str()).unwrap_or_default().into());
    let (reason, with) = state.notice.as_ref().map(|(r, w)| (end_reason(*r), w.as_str())).unwrap_or_default();
    ui.set_call_notice_reason(reason.into());
    ui.set_call_notice_with(with.into());

    let lists = state.lists.as_ref();
    sync(
        &models.friends,
        lists.map(|l| &l.friends[..]).unwrap_or_default().iter().map(|f| FriendRow {
            name: f.username.as_str().into(),
            presence: match f.presence {
                Presence::Online => "online",
                Presence::InCall => "in-call",
                Presence::Offline | Presence::Unknown => "offline",
            }
            .into(),
        }),
    );
    sync(&models.requests, lists.map(|l| &l.incoming[..]).unwrap_or_default().iter().map(|n| n.as_str().into()));
    sync(&models.pending, lists.map(|l| &l.outgoing[..]).unwrap_or_default().iter().map(|n| n.as_str().into()));
    sync(&models.blocked, lists.map(|l| &l.blocked[..]).unwrap_or_default().iter().map(|n| n.as_str().into()));
    sync(
        &models.missed,
        lists
            .map(|l| &l.missed[..])
            .unwrap_or_default()
            .iter()
            .map(|m| MissedRow { from: m.from.as_str().into(), at: date_time(m.at).into() }),
    );
    sync(
        &models.rooms,
        lists
            .map(|l| &l.rooms[..])
            .unwrap_or_default()
            .iter()
            .map(|r| RoomRow { room_id: r.room_id.as_str().into(), with: r.with.as_str().into() }),
    );
}

/// Replaces the rows that differ, so a list that didn't change isn't rebuilt under the pointer.
fn sync<T: Clone + PartialEq + 'static>(model: &ModelRc<T>, rows: impl Iterator<Item = T>) {
    let Some(vec) = model.as_any().downcast_ref::<VecModel<T>>() else { return };
    let rows: Vec<T> = rows.collect();
    if vec.iter().eq(rows.iter().cloned()) {
        return;
    }
    vec.set_vec(rows);
}

fn date_time(epoch_ms: u64) -> String {
    i64::try_from(epoch_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|at| at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The server's code, as `AccountText.error` in errors.slint takes it.
fn error_code(code: ApiErrorCode) -> &'static str {
    match code {
        ApiErrorCode::BadRequest | ApiErrorCode::NotFound | ApiErrorCode::Unknown => "bad-request",
        ApiErrorCode::Unauthorized => "unauthorized",
        ApiErrorCode::Forbidden => "forbidden",
        ApiErrorCode::RateLimited => "rate-limited",
        ApiErrorCode::Unavailable => "unavailable",
        ApiErrorCode::InvalidCredentials => "invalid-credentials",
        ApiErrorCode::InvalidInvite => "invalid-invite",
        ApiErrorCode::InvalidRecovery => "invalid-recovery",
        ApiErrorCode::UsernameTaken => "username-taken",
        ApiErrorCode::InvalidUsername => "invalid-username",
        ApiErrorCode::WeakPassword => "weak-password",
        ApiErrorCode::TooManyInvites => "too-many-invites",
        ApiErrorCode::TooMany => "too-many",
    }
}

/// As `CallOverlay`'s `notice` takes it; "" for what needs no notice.
fn end_reason(reason: CallEndReason) -> &'static str {
    match reason {
        CallEndReason::Declined => "declined",
        CallEndReason::Busy => "busy",
        CallEndReason::Offline => "offline",
        CallEndReason::Timeout => "timeout",
        CallEndReason::Unavailable => "unavailable",
        CallEndReason::Cancelled | CallEndReason::AnsweredElsewhere | CallEndReason::Unknown => "",
    }
}

fn session_path() -> Option<PathBuf> {
    Some(settings::data_dir()?.join("session"))
}

/// The token from the last sign-in, kept in its own file (not settings.json) so it never ends up in a log of the
/// settings.
fn load_session() -> Option<SessionToken> {
    let token = fs::read_to_string(session_path()?).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| SessionToken(token.to_owned()))
}

fn save_session(session: Option<&SessionToken>) {
    let Some(path) = session_path() else { return };
    let result = match session {
        Some(token) => path.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| write_private(&path, &token.0)),
        None => {
            fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        }
    };
    if let Err(e) = result {
        tracing::warn!("couldn't save the session: {e}");
    }
}

/// Readable by this user only, where the OS has such permissions.
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        file.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}
