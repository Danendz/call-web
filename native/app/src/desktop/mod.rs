//! The desktop app outside its window. With Settings' "Keep running when the window is closed" on and an account
//! signed in, closing the window hides the app to the tray or menu bar, where friends see it as away and their calls
//! still ring: with the ring tone, and a notification while the window isn't in front.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
mod tray;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

use std::cell::RefCell;
use std::rc::Rc;

use engine::friends::{Friends, FriendsCommand, FriendsState};
use engine::ringtone::Ringtone;
use engine::{Command, Engine};
use slint::winit_030::WinitWindowAccessor;
use slint::{CloseRequestResponse, ComponentHandle, Model};

use crate::settings::Settings;
use crate::ui::{AppPhase, AppWindow};
use crate::update::Updater;

/// Started by an update installed while hidden, so the new version comes back hidden too.
const START_HIDDEN: &str = "--hidden";

/// What a click on a notification asks for.
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Open,
    Answer(String),
    Decline(String),
}

/// The tray's and the notifications' text, from `tray-text` in app.slint.
pub(crate) struct Text {
    pub quit: String,
    pub calling_you: String,
    pub missed_call: String,
    pub accept: String,
    pub decline: String,
}

struct Desktop {
    ui: slint::Weak<AppWindow>,
    engine: Rc<Engine>,
    friends: Rc<Friends>,
    settings: Rc<RefCell<Settings>>,
    updater: Option<Updater>,
    text: Text,
    tray: Option<tray::Tray>,
    hidden: bool,
    focused: bool,
    signed_in: bool,
    /// The call ringing now, and its tone while it plays
    ringing: Option<(String, Option<Ringtone>)>,
    /// The missed calls already seen, so only new ones are notified; None until the first list
    missed: Option<Vec<(String, u64)>>,
    update_ready: bool,
}

thread_local! {
    static DESKTOP: RefCell<Option<Desktop>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Desktop) -> R) -> Option<R> {
    DESKTOP.with_borrow_mut(|desktop| desktop.as_mut().map(f))
}

/// Takes over the window's close button and shows the Settings switch. Call before the event loop runs.
pub fn attach(
    ui: &AppWindow,
    engine: &Rc<Engine>,
    friends: &Rc<Friends>,
    settings: &Rc<RefCell<Settings>>,
    updater: Option<Updater>,
) {
    let text = ui.get_tray_text();
    let at = |i| text.row_data(i).map(String::from).unwrap_or_default();
    let text = Text { quit: at(0), calling_you: at(1), missed_call: at(2), accept: at(3), decline: at(4) };
    let icon = ui.get_app_icon();

    ui.set_background_calls_kind("desktop".into());
    ui.set_background_calls(settings.borrow().keep_running);
    ui.on_set_background_calls({
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            let mut settings = settings.borrow_mut();
            settings.keep_running = on;
            settings.save();
            if let Some(ui) = weak.upgrade() {
                ui.set_background_calls(on);
            }
        }
    });

    ui.window().on_close_requested(|| {
        let keep = with(|d| d.signed_in && d.settings.borrow().keep_running).unwrap_or(false);
        if keep {
            hide();
        } else {
            slint::quit_event_loop().ok();
        }
        CloseRequestResponse::HideWindow
    });

    DESKTOP.with_borrow_mut(|desktop| {
        *desktop = Some(Desktop {
            ui: ui.as_weak(),
            engine: engine.clone(),
            friends: friends.clone(),
            settings: settings.clone(),
            updater,
            text,
            tray: None,
            hidden: false,
            focused: true,
            signed_in: false,
            ringing: None,
            missed: None,
            update_ready: false,
        });
    });
    // The tray and the notification center need the app's event loop running (NSApplication on macOS)
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        with(|d| {
            d.tray = tray::Tray::new(&d.text, &icon);
            platform::start(&d.text);
        });
    });
}

/// Shows the window, unless this launch came from an update installed while hidden; then runs until quit.
pub fn run(ui: &AppWindow) -> Result<(), slint::PlatformError> {
    if std::env::args().any(|arg| arg == START_HIDDEN) {
        with(|d| d.hidden = true);
        platform::set_in_dock(false);
    } else {
        ui.show()?;
    }
    slint::run_event_loop_until_quit()
}

/// Drops the tray icon and stops ringing, before the process ends.
pub fn detach() {
    DESKTOP.with_borrow_mut(|desktop| *desktop = None);
}

/// The window came to the front or left it (winit's `Focused`).
pub fn focused(focused: bool) {
    with(|d| d.focused = focused);
}

/// The friends thread's new state: rings and missed calls become sound and notifications.
pub fn friends_state(state: &FriendsState) {
    with(|d| {
        d.signed_in = matches!(state.account, engine::friends::Account::SignedIn { .. });
        let in_front = !d.hidden && d.focused;

        let incoming = state.incoming.first();
        if d.ringing.as_ref().map(|(id, _)| id) != incoming.map(|c| &c.call_id) {
            if let Some((id, _)) = d.ringing.take() {
                platform::remove(&id);
            }
            if let Some(call) = incoming {
                if !in_front {
                    platform::incoming(&d.text, &call.call_id, &call.from);
                }
                d.ringing = Some((call.call_id.clone(), Ringtone::start()));
            }
        }

        if let Some(lists) = &state.lists {
            let now: Vec<(String, u64)> = lists.missed.iter().map(|m| (m.from.clone(), m.at)).collect();
            if let Some(known) = &d.missed
                && !in_front
            {
                for (from, _) in now.iter().filter(|m| !known.contains(m)) {
                    platform::missed(&d.text, from);
                }
            }
            d.missed = Some(now);
        }
    });
}

/// An update finished downloading: installed now if the app is hidden and out of a call, else on quit.
pub fn update_ready() {
    with(|d| d.update_ready = true);
    install_if_hidden();
}

/// A notification's button, from the OS on whatever thread it calls back on.
pub(super) fn deliver(action: Action) {
    let _ = slint::invoke_from_event_loop(move || act(action));
}

fn act(action: Action) {
    match action {
        Action::Open => show(),
        Action::Answer(call_id) => {
            with(|d| d.friends.send(FriendsCommand::Answer { call_id, accept: true }));
            show();
        }
        Action::Decline(call_id) => {
            with(|d| d.friends.send(FriendsCommand::Answer { call_id, accept: false }));
        }
    }
}

/// Hides the window to the tray. A call can't go on unseen, so closing the window ends it.
fn hide() {
    with(|d| {
        let Some(ui) = d.ui.upgrade() else { return };
        if matches!(ui.get_phase(), AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall) {
            d.engine.send(Command::HangUp);
        }
        d.hidden = true;
        d.friends.send(FriendsCommand::Foreground(false));
        platform::set_in_dock(false);
    });
    install_if_hidden();
}

pub(crate) fn show() {
    with(|d| {
        let Some(ui) = d.ui.upgrade() else { return };
        if let Err(e) = ui.show() {
            tracing::warn!("showing the window: {e}");
            return;
        }
        if d.hidden {
            d.hidden = false;
            d.friends.send(FriendsCommand::Foreground(true));
        }
        platform::set_in_dock(true);
        platform::bring_to_front();
        ui.window().with_winit_window(|window| {
            window.set_minimized(false);
            window.focus_window();
        });
    });
}

fn install_if_hidden() {
    let updater = with(|d| {
        let in_call =
            d.ui.upgrade()
                .is_some_and(|ui| matches!(ui.get_phase(), AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall));
        (d.hidden && d.update_ready && !in_call && d.ringing.is_none()).then(|| d.updater.clone()).flatten()
    })
    .flatten();
    if let Some(updater) = updater {
        tracing::info!("installing the update while hidden");
        updater.restart_now_with(&[START_HIDDEN]);
    }
}
