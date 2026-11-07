// No console window behind the app on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod window;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context;
use engine::{Command, Engine, Lang, Link, Phase, State, room};
use settings::Settings;
use url::Url;

// Slint's generated code isn't ours to lint
#[allow(clippy::all, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::todo)]
mod ui {
    slint::include_modules!();
}
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use ui::{AppPhase, AppWindow, CallLink, DeviceItem};

const DEFAULT_SERVER: &str = "https://call.danendz.com";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let server = server_url()?;
    window::select_backend().context("no usable graphics backend")?;

    let settings = Rc::new(RefCell::new(Settings::load()));
    let lang = settings.borrow().lang.unwrap_or_else(settings::system_lang);
    let ui = AppWindow::new()?;
    // Only works once a component exists
    select_translation(lang);
    ui.set_lang(lang_code(lang).into());

    let mics = Rc::new(VecModel::<DeviceItem>::default());
    ui.set_mics(ModelRc::from(mics.clone()));
    // The call timer ticks on the UI thread from the start time the engine reports
    let call_started = Rc::new(RefCell::new(None::<Instant>));

    let weak = ui.as_weak();
    let engine = Rc::new(Engine::start(server, lang, move |state| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            // Only the UI thread touches the models and the timer's start; the engine thread just hands state over
            BRIDGE.with(|bridge| {
                if let Some(bridge) = bridge.borrow().as_ref() {
                    *bridge.call_started.borrow_mut() = state.call_started;
                    sync_devices(&bridge.mics, &state);
                }
            });
            render(&ui, &state);
        });
    })?);
    BRIDGE.with(|bridge| *bridge.borrow_mut() = Some(Bridge { mics, call_started: call_started.clone() }));

    // Things that change continuously are read on the UI's clock instead of being pushed: the call timer, and the
    // mic level while the setup screen shows it
    let ticker = slint::Timer::default();
    ticker.start(slint::TimerMode::Repeated, Duration::from_millis(33), {
        let weak = ui.as_weak();
        let engine = engine.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let text = call_started.borrow().map(|at| format_duration(at.elapsed())).unwrap_or_default();
            if ui.get_elapsed() != text.as_str() {
                ui.set_elapsed(text.into());
            }
            let level = if ui.get_phase() == AppPhase::Setup { engine.mic_level() } else { 0.0 };
            if (ui.get_mic_level() - level).abs() > 0.005 {
                ui.set_mic_level(level);
            }
        }
    });
    if let Some(mic) = settings.borrow().mic.clone() {
        engine.send(Command::SelectMic(mic));
    }

    ui.on_new_room({
        let engine = engine.clone();
        move || engine.send(Command::NewRoom)
    });
    ui.on_open_link({
        let engine = engine.clone();
        move |link| match room::parse_room(&link) {
            Some(id) => {
                engine.send(Command::OpenRoom(id));
                true
            }
            None => false,
        }
    });
    ui.on_set_lang({
        let engine = engine.clone();
        let settings = settings.clone();
        move |code| {
            let lang = if code == "zh" { Lang::Zh } else { Lang::En };
            select_translation(lang);
            let mut settings = settings.borrow_mut();
            settings.lang = Some(lang);
            settings.save();
            engine.send(Command::SetLang(lang));
        }
    });
    ui.on_join({
        let engine = engine.clone();
        move || engine.send(Command::Join)
    });
    ui.on_hang_up({
        let engine = engine.clone();
        move || engine.send(Command::HangUp)
    });
    ui.on_home({
        let engine = engine.clone();
        move || engine.send(Command::Home)
    });
    ui.on_toggle_mic({
        let engine = engine.clone();
        move || engine.send(Command::ToggleMic)
    });
    ui.on_pick_mic({
        let engine = engine.clone();
        move |id| {
            let mut settings = settings.borrow_mut();
            settings.mic = Some(id.to_string());
            settings.save();
            engine.send(Command::SelectMic(id.into()));
        }
    });

    ui.run()?;
    // Dropping the engine leaves the room before the process ends
    drop(ui);
    drop(engine);
    Ok(())
}

/// `CALL_SERVER` points the app at another server, e.g. `http://localhost:8090` from `PORT=8090 npm run dev:server`.
fn server_url() -> anyhow::Result<Url> {
    let raw = std::env::var("CALL_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let url = Url::parse(&raw).with_context(|| format!("CALL_SERVER isn't a URL: {raw}"))?;
    anyhow::ensure!(matches!(url.scheme(), "http" | "https"), "CALL_SERVER must be http or https: {raw}");
    Ok(url)
}

/// UI-thread state the engine callback reaches through a thread-local, since models aren't `Send`.
struct Bridge {
    mics: Rc<VecModel<DeviceItem>>,
    call_started: Rc<RefCell<Option<Instant>>>,
}

thread_local! {
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

fn render(ui: &AppWindow, state: &State) {
    ui.set_phase(match state.phase {
        Phase::Start => AppPhase::Start,
        Phase::Setup => AppPhase::Setup,
        Phase::Starting => AppPhase::Starting,
        Phase::Waiting => AppPhase::Waiting,
        Phase::InCall => AppPhase::InCall,
        Phase::Full => AppPhase::Full,
        Phase::Ended => AppPhase::Ended,
    });
    ui.set_lang(lang_code(state.lang).into());
    ui.set_room_url(state.room_url.clone().unwrap_or_default().into());
    ui.set_reconnecting(state.reconnecting);
    ui.set_link(match state.link {
        None => CallLink::None,
        Some(Link::Connecting) => CallLink::Connecting,
        Some(Link::Connected) => CallLink::Connected,
        Some(Link::Unstable) => CallLink::Unstable,
        Some(Link::Failed) => CallLink::Failed,
    });
    ui.set_mic_on(state.mic_on);
    // Nothing picked means the system default, which the OS lists first
    let mic = state.mic_id.as_ref().and_then(|id| state.mics.iter().find(|d| &d.id == id)).or(state.mics.first());
    ui.set_mic_id(mic.map(|d| d.id.as_str()).unwrap_or_default().into());
    ui.set_mic_label(mic.map(|d| d.label.as_str()).unwrap_or_default().into());
}

/// Updated in place, and only when the list really changed, so an open picker isn't rebuilt under the pointer.
fn sync_devices(model: &VecModel<DeviceItem>, state: &State) {
    let same = model.row_count() == state.mics.len()
        && model.iter().zip(&state.mics).all(|(row, d)| row.id == d.id.as_str() && row.label == d.label.as_str());
    if !same {
        model.set_vec(
            state
                .mics
                .iter()
                .map(|d| DeviceItem { id: SharedString::from(&d.id), label: SharedString::from(&d.label) })
                .collect::<Vec<_>>(),
        );
    }
}

/// 4:05, or 1:04:05 past an hour, like the web's call timer.
fn format_duration(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
}

/// English is the source language; Chinese comes from translations/zh, bundled into the binary.
fn select_translation(lang: Lang) {
    if let Err(e) = slint::select_bundled_translation(lang_code(lang)) {
        tracing::warn!("no translation for {}: {e}", lang_code(lang));
    }
}
