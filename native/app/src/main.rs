// No console window behind the app on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod chat;
mod settings;
mod video;
mod window;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context;
use bridge::Bridge;
use engine::{Command, Engine, Lang, room};
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

thread_local! {
    // The engine's state callback reaches the UI-thread bridge through this: models aren't `Send`
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

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

    let bridge = Bridge::new(&ui, lang);
    let call_started = bridge.call_started.clone();
    BRIDGE.with(|b| *b.borrow_mut() = Some(bridge));

    let weak = ui.as_weak();
    let sink = video::SlintSink::new(ui.as_weak());
    let chat_weak = ui.as_weak();
    let engine = Rc::new(Engine::start(
        server,
        lang,
        sink,
        move |state| {
            let _ = weak.upgrade_in_event_loop(move |ui| with_bridge(|bridge| bridge.render(&ui, &state)));
        },
        move |event| {
            let _ = chat_weak.upgrade_in_event_loop(move |ui| with_bridge(|bridge| bridge.chat(&ui, event)));
        },
    )?);

    // Things that change continuously are read on the UI's clock instead of being pushed: the call timer, and the
    // mic level while the setup screen shows it
    let ticker = slint::Timer::default();
    ticker.start(slint::TimerMode::Repeated, Duration::from_millis(33), {
        let weak = ui.as_weak();
        let engine = engine.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let text = call_started.borrow().map(|at| bridge::format_duration(at.elapsed())).unwrap_or_default();
            if ui.get_elapsed() != text.as_str() {
                ui.set_elapsed(text.into());
            }
            let level = if ui.get_phase() == AppPhase::Setup { engine.mic_level() } else { 0.0 };
            if (ui.get_mic_level() - level).abs() > 0.005 {
                ui.set_mic_level(level);
            }
        }
    });
    {
        let settings = settings.borrow();
        if let Some(mic) = settings.mic.clone() {
            engine.send(Command::SelectMic(mic));
        }
        if let Some(cam) = settings.cam.clone() {
            engine.send(Command::SelectCam(cam));
        }
        if settings.portrait {
            engine.send(Command::SetPortrait(true));
        }
        if settings.transcribe {
            engine.send(Command::SetTranscribe(true));
        }
    }

    let send = |command: Command| {
        let engine = engine.clone();
        move || engine.send(command.clone())
    };
    ui.on_new_room(send(Command::NewRoom));
    ui.on_join(send(Command::Join));
    ui.on_hang_up(send(Command::HangUp));
    ui.on_home(send(Command::Home));
    ui.on_toggle_mic(send(Command::ToggleMic));
    ui.on_toggle_cam(send(Command::ToggleCam));
    ui.on_next_cam(send(Command::NextCam));
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
    ui.on_send_chat({
        let engine = engine.clone();
        move |text| {
            engine.send(Command::SendChat(text.into()));
            // The box clears; the message shows once the server echoes it back, as on the web
            true
        }
    });
    ui.on_toggle_transcribe({
        let engine = engine.clone();
        let settings = settings.clone();
        let weak = ui.as_weak();
        move || {
            let on = !weak.upgrade().is_some_and(|ui| ui.get_transcribe_on());
            let mut settings = settings.borrow_mut();
            settings.transcribe = on;
            settings.save();
            engine.send(Command::SetTranscribe(on));
        }
    });
    ui.on_set_force_relay({
        let engine = engine.clone();
        move |relay| engine.send(Command::SetForceRelay(relay))
    });
    // Choices that are remembered for next time
    let remember = |change: fn(&mut Settings, String), command: fn(String) -> Command| {
        let engine = engine.clone();
        let settings = settings.clone();
        move |value: slint::SharedString| {
            let mut settings = settings.borrow_mut();
            change(&mut settings, value.to_string());
            settings.save();
            engine.send(command(value.into()));
        }
    };
    ui.on_pick_mic(remember(|s, id| s.mic = Some(id), Command::SelectMic));
    ui.on_pick_cam(remember(|s, id| s.cam = Some(id), Command::SelectCam));
    ui.on_set_lang(remember(
        |s, code| s.lang = Some(parse_lang(&code)),
        |code| {
            let lang = parse_lang(&code);
            select_translation(lang);
            Command::SetLang(lang)
        },
    ));
    ui.on_set_portrait({
        let engine = engine.clone();
        let settings = settings.clone();
        move |portrait| {
            let mut settings = settings.borrow_mut();
            settings.portrait = portrait;
            settings.save();
            engine.send(Command::SetPortrait(portrait));
        }
    });

    ui.run()?;
    // Dropping the engine leaves the room before the process ends
    drop(ui);
    drop(engine);
    Ok(())
}

fn with_bridge(f: impl FnOnce(&Bridge)) {
    BRIDGE.with(|bridge| {
        if let Some(bridge) = bridge.borrow().as_ref() {
            f(bridge);
        }
    });
}

fn parse_lang(code: &str) -> Lang {
    if code == "zh" { Lang::Zh } else { Lang::En }
}

/// `CALL_SERVER` points the app at another server, e.g. `http://localhost:8090` from `PORT=8090 npm run dev:server`.
fn server_url() -> anyhow::Result<Url> {
    let raw = std::env::var("CALL_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let url = Url::parse(&raw).with_context(|| format!("CALL_SERVER isn't a URL: {raw}"))?;
    anyhow::ensure!(matches!(url.scheme(), "http" | "https"), "CALL_SERVER must be http or https: {raw}");
    Ok(url)
}

pub fn lang_code(lang: Lang) -> &'static str {
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
