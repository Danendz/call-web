// No console window behind the app on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Context;
use engine::{Command, Engine, Lang, Phase, State, room};
use settings::Settings;
use url::Url;

// Slint's generated code isn't ours to lint
#[allow(clippy::all, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::todo)]
mod ui {
    slint::include_modules!();
}
use slint::ComponentHandle;
use ui::{AppPhase, AppWindow};

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

    let weak = ui.as_weak();
    let engine = Rc::new(Engine::start(server, lang, move |state| {
        let _ = weak.upgrade_in_event_loop(move |ui| render(&ui, &state));
    })?);

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

fn render(ui: &AppWindow, state: &State) {
    ui.set_phase(match state.phase {
        Phase::Start => AppPhase::Start,
        Phase::Setup => AppPhase::Setup,
        Phase::Starting => AppPhase::Starting,
        Phase::Waiting => AppPhase::Waiting,
        Phase::Full => AppPhase::Full,
        Phase::Ended => AppPhase::Ended,
    });
    ui.set_lang(lang_code(state.lang).into());
    ui.set_room_url(state.room_url.clone().unwrap_or_default().into());
    ui.set_peer_present(state.peer_present);
    ui.set_reconnecting(state.reconnecting);
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
