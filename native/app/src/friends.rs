//! The account and friends: the friends thread's state into the window, the window's callbacks into commands, and
//! the session token kept between launches.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, Local};
use engine::friends::{Account, FriendAction, Friends, FriendsCommand, FriendsEvent, FriendsState, SessionToken};
use engine::{ApiErrorCode, CallEndReason, Command, Engine, FriendState, MissedCall, Presence};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::settings;
use crate::ui::{AppWindow, FriendRow, RoomRow};

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
                #[cfg(target_os = "android")]
                crate::android::session_changed();
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
    ui.on_missed_seen({
        let friends = friends.clone();
        move |from| friends.send(FriendsCommand::MissedSeen(from.into()))
    });
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
                #[cfg(target_os = "ios")]
                crate::ios::call::answered(accept);
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
    rooms: ModelRc<RoomRow>,
}

impl Models {
    fn new(ui: &AppWindow) -> Self {
        let models = Self {
            friends: ModelRc::from(Rc::new(VecModel::default())),
            requests: ModelRc::from(Rc::new(VecModel::default())),
            pending: ModelRc::from(Rc::new(VecModel::default())),
            blocked: ModelRc::from(Rc::new(VecModel::default())),
            rooms: ModelRc::from(Rc::new(VecModel::default())),
        };
        ui.set_friends(models.friends.clone());
        ui.set_friend_requests(models.requests.clone());
        ui.set_pending_requests(models.pending.clone());
        ui.set_blocked(models.blocked.clone());
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
    #[cfg(desktop)]
    crate::desktop::friends_state(state);
    #[cfg(target_os = "android")]
    crate::android::answer_from_notification(ui, state);
    ui.set_friends_connected(state.connected);
    ui.set_friends_loaded(state.lists.is_some());
    ui.set_calling(state.outgoing.as_ref().map(|o| o.to.as_str()).unwrap_or_default().into());
    ui.set_calling_offline(state.outgoing.as_ref().is_some_and(|o| o.presence == Some(Presence::Offline)));
    let incoming = state.incoming.first();
    ui.set_incoming_from(incoming.map(|c| c.from.as_str()).unwrap_or_default().into());
    ui.set_incoming_call_id(incoming.map(|c| c.call_id.as_str()).unwrap_or_default().into());
    let (reason, with) = state.notice.as_ref().map(|(r, w)| (end_reason(*r), w.as_str())).unwrap_or_default();
    ui.set_call_notice_reason(reason.into());
    ui.set_call_notice_with(with.into());

    let lists = state.lists.as_ref();
    let mut friends = friend_rows(lists.map(|l| (&l.friends[..], &l.missed[..])).unwrap_or_default());
    for row in &mut friends {
        let was = models.friends.iter().find(|old| old.name == row.name).map(|old| old.presence);
        row.came_online = row.presence == "online" && was.is_some_and(|was| was != "online");
    }
    sync(&models.friends, friends.into_iter(), |r| r.name.clone());
    let name = |n: &SharedString| n.clone();
    sync(&models.requests, lists.map(|l| &l.incoming[..]).unwrap_or_default().iter().map(|n| n.as_str().into()), name);
    sync(&models.pending, lists.map(|l| &l.outgoing[..]).unwrap_or_default().iter().map(|n| n.as_str().into()), name);
    sync(&models.blocked, lists.map(|l| &l.blocked[..]).unwrap_or_default().iter().map(|n| n.as_str().into()), name);
    sync(
        &models.rooms,
        lists
            .map(|l| &l.rooms[..])
            .unwrap_or_default()
            .iter()
            .map(|r| RoomRow { room_id: r.room_id.as_str().into(), with: r.with.as_str().into() }),
        |r| r.room_id.clone(),
    );
}

/// Friends with a missed call first (the latest on top), then online, away, in a call and offline, each by name, like the
/// web's `sortFriends`. Missed calls from someone who isn't a friend aren't shown.
fn friend_rows((friends, missed): (&[FriendState], &[MissedCall])) -> Vec<FriendRow> {
    let mut rows: Vec<(u64, u8, FriendRow)> = friends
        .iter()
        .map(|f| {
            let from_them = missed.iter().filter(|m| m.from == f.username);
            let (count, last) = from_them.fold((0, 0), |(n, last), m| (n + 1, last.max(m.at)));
            let (presence, order) = match f.presence {
                Presence::Online => ("online", 0),
                Presence::Away => ("away", 1),
                Presence::InCall => ("in-call", 2),
                Presence::Offline | Presence::Unknown => ("offline", 3),
            };
            let row = FriendRow {
                name: f.username.as_str().into(),
                presence: presence.into(),
                missed: count,
                missed_at: if count > 0 { date_time(last).into() } else { SharedString::default() },
                came_online: false,
            };
            (last, order, row)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then_with(|| a.2.name.cmp(&b.2.name)));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

/// Brings the model to `rows` by moving, adding and removing rows by `key` and replacing only the rows that changed,
/// so a row that stays keeps its element (only the ones that come in or move fade in) and a list that didn't change
/// isn't rebuilt under the pointer.
fn sync<T: Clone + PartialEq + 'static, K: PartialEq>(
    model: &ModelRc<T>,
    rows: impl Iterator<Item = T>,
    key: impl Fn(&T) -> K,
) {
    let Some(vec) = model.as_any().downcast_ref::<VecModel<T>>() else { return };
    let rows: Vec<T> = rows.collect();
    let key_at = |i: usize| vec.row_data(i).map(|old| key(&old));
    for (i, row) in rows.iter().enumerate() {
        let k = key(row);
        // The row here is gone, or moved down past this one: it comes back in further down
        while key_at(i).is_some_and(|old| {
            old != k && (key_at(i + 1).as_ref() == Some(&k) || !rows.iter().skip(i).any(|r| key(r) == old))
        }) {
            vec.remove(i);
        }
        match (i..vec.row_count()).find(|&j| key_at(j).as_ref() == Some(&k)) {
            Some(j) if j > i => {
                vec.remove(j);
                vec.insert(i, row.clone());
            }
            Some(_) if vec.row_data(i).as_ref() != Some(row) => vec.set_row_data(i, row.clone()),
            Some(_) => {}
            None => vec.insert(i, row.clone()),
        }
    }
    while vec.row_count() > rows.len() {
        vec.remove(rows.len());
    }
}

fn date_time(epoch_ms: u64) -> String {
    i64::try_from(epoch_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|at| at.with_timezone(&Local))
        // Just the time for today's calls, so the tag fits on a phone
        .map(|at| {
            at.format(if at.date_naive() == Local::now().date_naive() { "%H:%M" } else { "%Y-%m-%d %H:%M" }).to_string()
        })
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
pub(crate) fn load_session() -> Option<SessionToken> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(from: &[&str], to: &[&str]) -> Vec<String> {
        let model: ModelRc<SharedString> = ModelRc::from(Rc::new(VecModel::from_iter(from.iter().map(|&n| n.into()))));
        sync(&model, to.iter().map(|&n| n.into()), |n| n.clone());
        model.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn sync_reaches_the_new_order() {
        for (from, to) in [
            (&["a", "b", "c", "d"][..], &["b", "c", "d", "a"][..]),
            (&["a", "b", "c", "d"], &["d", "a", "b", "c"]),
            (&["a", "b"], &["b", "a"]),
            (&["a", "b", "c"], &["a", "c"]),
            (&["a", "c"], &["a", "b", "c", "e"]),
            (&["a", "b", "c"], &[]),
            (&[], &["c", "a"]),
            (&["x", "a", "y", "b"], &["b", "z", "a"]),
        ] {
            assert_eq!(synced(from, to), to, "{from:?} -> {to:?}");
        }
    }
}
