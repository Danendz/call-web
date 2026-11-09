//! The app: the Slint UI and the bridge from engine state to it, started by `main.rs` on desktop and by
//! `android_main` on Android.

#[cfg(target_os = "android")]
mod android;
mod bridge;
mod chat;
mod screenshot;
mod settings;
#[cfg(not(target_os = "android"))]
mod update;
mod video;
#[cfg(not(target_os = "android"))]
pub mod window;

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

// A debug build talks to the local dev server (`PORT=8090 npm run dev:server`), so its rooms and copied links
// stay off production
const DEFAULT_SERVER: &str = if cfg!(debug_assertions) { "http://localhost:8090" } else { "https://call.danendz.com" };

thread_local! {
    // The engine's state callback reaches the UI-thread bridge through this: models aren't `Send`
    #[cfg_attr(target_os = "android", allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android"))]
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

/// Runs the app until its window closes. The caller has set up logging and Slint's backend.
pub fn run() -> anyhow::Result<()> {
    let server = server_url()?;

    let settings = Rc::new(RefCell::new(Settings::load()));
    let lang = settings.borrow().lang.unwrap_or_else(settings::system_lang);
    let ui = AppWindow::new()?;
    // Only works once a component exists
    select_translation(lang);
    ui.set_lang(lang_code(lang).into());
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_touch(cfg!(target_os = "android"));

    #[cfg(target_os = "android")]
    android::attach(&ui, &settings);

    let bridge = Bridge::new(&ui, lang);
    let call_started = bridge.call_started.clone();
    BRIDGE.with(|b| *b.borrow_mut() = Some(bridge));

    #[cfg(not(target_os = "android"))]
    let updater = update::Updater::start(
        &server,
        {
            let weak = ui.as_weak();
            move |percent| {
                let _ = weak.upgrade_in_event_loop(move |ui| ui.set_update_progress(percent.into()));
            }
        },
        {
            let weak = ui.as_weak();
            move |version| {
                let _ = weak.upgrade_in_event_loop(move |ui| ui.set_update_version(version.into()));
            }
        },
    );
    #[cfg(not(target_os = "android"))]
    ui.on_update_now({
        let updater = updater.clone();
        move || {
            if let Some(updater) = &updater {
                updater.restart_now();
            }
        }
    });

    #[cfg(target_os = "android")]
    android::updates(&ui, &server);

    let weak = ui.as_weak();
    let sink = video::SlintSink::new(ui.as_weak());
    let chat_weak = ui.as_weak();
    let (on_device, data_dir) = (settings.borrow().on_device_speech, settings::data_dir());
    let engine = Rc::new(Engine::start(
        server,
        lang,
        data_dir.clone(),
        on_device,
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
            #[cfg(target_os = "android")]
            android::sync(&ui);
        }
    });
    {
        let settings = settings.borrow();
        if let Some(mic) = settings.mic.clone() {
            engine.send(Command::SelectMic(mic));
        }
        if let Some(speaker) = settings.speaker.clone() {
            engine.send(Command::SelectSpeaker(speaker));
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
        engine.send(Command::SetNoiseSuppression(settings.noise_suppression));
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
    ui.on_toggle_speaker(send(Command::ToggleSpeaker));
    ui.on_toggle_cam(send(Command::ToggleCam));
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
    ui.on_screenshot({
        let engine = engine.clone();
        let weak = ui.as_weak();
        move |feed, rotation| {
            let Some(ui) = weak.upgrade() else { return };
            let main = match feed.as_str() {
                "screen" => ui.get_screen_video(),
                "self" => ui.get_self_video(),
                _ => ui.get_peer_video(),
            };
            let Some(main) = main.to_rgba8() else { return };
            // Your own camera rides along as a tile, unless it's what's on the stage
            let tile = (feed != "self" && ui.get_cam_on()).then(|| ui.get_self_video().to_rgba8()).flatten();
            let picture = screenshot::Picture {
                main: screenshot::Shot { frame: main, rotation },
                tile: tile.map(|frame| screenshot::Shot { frame, rotation: ui.get_self_frame_rotation() }),
                elapsed: ui.get_elapsed().to_string(),
                lang: parse_lang(&ui.get_lang()),
            };
            engine.send(Command::NotifyScreenshot);
            let weak = ui.as_weak();
            // Encoding takes a moment; the window doesn't wait for it
            std::thread::spawn(move || match screenshot::save(&picture) {
                Ok(saved) => {
                    let _ = weak.upgrade_in_event_loop(move |ui| ui.set_saved_shot(saved.into()));
                }
                Err(e) => {
                    tracing::warn!("screenshot failed: {e}");
                    let _ = weak.upgrade_in_event_loop(|ui| ui.set_shot_failures(ui.get_shot_failures() + 1));
                }
            });
        }
    });
    ui.on_show_shot({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                screenshot::show(ui.get_saved_shot().as_str());
            }
        }
    });
    ui.on_toggle_share(send(Command::ToggleShare));
    ui.on_close_screen_picker(send(Command::CloseScreenPicker));
    ui.on_share_source({
        let engine = engine.clone();
        move |id| engine.send(Command::ShareSource(id.into()))
    });
    {
        let settings = settings.borrow();
        for (feed, corner) in &settings.corners {
            set_corner(&ui, feed, corner);
        }
    }
    ui.on_corner_moved({
        let weak = ui.as_weak();
        let settings = settings.clone();
        move |feed, corner| {
            let Some(ui) = weak.upgrade() else { return };
            set_corner(&ui, &feed, &corner);
            let mut settings = settings.borrow_mut();
            settings.corners.insert(feed.into(), corner.into());
            settings.save();
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
    ui.on_pick_speaker(remember(|s, id| s.speaker = Some(id), Command::SelectSpeaker));
    ui.on_pick_cam(remember(|s, id| s.cam = Some(id), Command::SelectCam));
    ui.on_set_lang(remember(
        |s, code| s.lang = Some(parse_lang(&code)),
        |code| {
            let lang = parse_lang(&code);
            select_translation(lang);
            Command::SetLang(lang)
        },
    ));
    ui.set_model_size(format!("{} MB", engine::MODEL_SIZE.div_ceil(1_000_000)).into());
    // Asked once, where there's somewhere to keep the model
    ui.set_speech_prompt(data_dir.is_some() && !settings.borrow().speech_prompt_seen);
    ui.on_dismiss_speech_prompt({
        let settings = settings.clone();
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_speech_prompt(false);
            }
            let mut settings = settings.borrow_mut();
            settings.speech_prompt_seen = true;
            settings.save();
        }
    });
    ui.on_download_model(send(Command::DownloadModel));
    ui.on_cancel_model_download(send(Command::CancelModelDownload));
    ui.on_delete_model(send(Command::DeleteModel));
    ui.on_set_on_device({
        let engine = engine.clone();
        let settings = settings.clone();
        move |on| {
            let mut settings = settings.borrow_mut();
            settings.on_device_speech = on;
            settings.save();
            engine.send(Command::SetOnDevice(on));
        }
    });
    ui.set_show_stats(settings.borrow().show_stats);
    ui.on_set_show_stats({
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_stats(on);
            }
            let mut settings = settings.borrow_mut();
            settings.show_stats = on;
            settings.save();
        }
    });
    ui.set_noise_suppression(settings.borrow().noise_suppression);
    ui.on_set_noise_suppression({
        let engine = engine.clone();
        let settings = settings.clone();
        let weak = ui.as_weak();
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_noise_suppression(on);
            }
            let mut settings = settings.borrow_mut();
            settings.noise_suppression = on;
            settings.save();
            engine.send(Command::SetNoiseSuppression(on));
        }
    });
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
    #[cfg(not(target_os = "android"))]
    if let Some(updater) = updater {
        updater.install_on_exit();
    }
    Ok(())
}

/// Where a tile sits; anything but the four corners is ignored, so a hand-edited settings file can't misplace one.
fn set_corner(ui: &AppWindow, feed: &str, corner: &str) {
    if !matches!(corner, "tl" | "tr" | "bl" | "br") {
        return;
    }
    match feed {
        "self" => ui.set_corner_self(corner.into()),
        "peer" => ui.set_corner_peer(corner.into()),
        "screen" => ui.set_corner_screen(corner.into()),
        _ => {}
    }
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

/// `CALL_SERVER` points the app at another server, e.g. production from a debug build.
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
