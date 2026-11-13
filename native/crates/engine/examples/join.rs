//! Joins a room without the UI and logs every state change, for checking calls when the app can't be driven (a
//! locked screen, CI) or as the second person in a native ↔ native or native ↔ browser test:
//!
//!   cargo run -p engine --example join -- http://localhost:8090 <room id or link> [seconds]
//!
//! With `CALL_FAKE_CAMERA=1` it sends a test pattern as its camera; video it receives is logged every few seconds,
//! and chat as it arrives. `SAY="hello"` posts that to the chat once the other person is there; `TRANSCRIBE=1` turns
//! speech to chat on (with `CALL_FAKE_MIC=<file.wav>` it speaks that file). `SHARE=1` shares the screen once in the
//! call (with `CALL_FAKE_SCREEN=1` a test pattern). `DATA_DIR=<folder>` recognizes speech on this machine with the
//! model in `<folder>/models/`, as the app does once it's downloaded; with `DOWNLOAD=1` it downloads the model there
//! first, from the server. `LIVE=1` turns live subtitles on, so speech goes to the live model there instead and is
//! sent as live text while it's said.

// A dev tool: bad arguments just stop it
#![allow(clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use engine::{ChatEvent, Command, Engine, Feed, Frame, Lang, Model, Phase, VideoSink, room};
use url::Url;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut args = std::env::args().skip(1);
    let (Some(server), Some(link)) = (args.next(), args.next()) else {
        eprintln!("usage: join <server> <room id or link> [seconds]");
        std::process::exit(2);
    };
    let server = Url::parse(&server).unwrap_or_else(|e| panic!("bad server url: {e}"));
    let room = room::parse_room(&link).unwrap_or_else(|| panic!("not a room: {link}"));
    let seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);

    let said = std::sync::atomic::AtomicBool::new(false);
    let (say_tx, say_rx) = std::sync::mpsc::channel::<String>();
    // Something has to wake the in-call wait below when only sharing
    let say = std::env::var("SAY").ok().or_else(|| std::env::var_os("SHARE").map(|_| String::new()));
    let on_chat = |event: ChatEvent| match event {
        ChatEvent::Reset { messages, .. } => tracing::info!("chat: {} earlier messages", messages.len()),
        ChatEvent::Added(m) => tracing::info!("chat from {}: {}", m.from, m.text),
        ChatEvent::Translated { id, translation } => tracing::info!("chat {id} translated: {:?}", translation.text),
        ChatEvent::Live { id, text } => tracing::info!("live {id}: {text}"),
        ChatEvent::Reaction { emoji, mine } => tracing::info!("reaction {} (mine: {mine})", emoji.emoji()),
    };
    let engine = Engine::start(
        server,
        Lang::En,
        std::env::var_os("DATA_DIR").map(Into::into),
        true,
        Arc::new(LogSink::default()),
        move |state| {
            if state.phase == Phase::InCall
                && let Some(text) = &say
                && !said.swap(true, Ordering::Relaxed)
            {
                let _ = say_tx.send(text.clone());
            }
            tracing::info!(
                "phase={:?} link={:?} reconnecting={} in_call_for={:?} mics={} model={:?} live={:?}",
                state.phase,
                state.link,
                state.reconnecting,
                state.call_started.map(|at| at.elapsed().as_secs()),
                state.mics.len(),
                state.model,
                state.live
            );
        },
        on_chat,
    )
    .unwrap_or_else(|e| panic!("engine: {e}"));
    let live = std::env::var_os("LIVE").is_some();
    engine.send(Command::SetLiveSubtitles(live));
    if std::env::var_os("DOWNLOAD").is_some() {
        engine.send(Command::DownloadModel(if live { Model::Live } else { Model::SenseVoice }));
    }
    engine.send(Command::OpenRoom(room));
    if std::env::var_os("TRANSCRIBE").is_some() {
        engine.send(Command::SetTranscribe(true));
    }
    engine.send(Command::Join);
    if let Ok(text) = say_rx.recv_timeout(Duration::from_secs(seconds)) {
        // A moment for the other side to settle into the call
        std::thread::sleep(Duration::from_secs(1));
        if !text.is_empty() {
            engine.send(Command::SendChat(text));
        }
        if std::env::var_os("SHARE").is_some() {
            engine.send(Command::ToggleShare);
        }
    }
    std::thread::sleep(Duration::from_secs(seconds));
    engine.send(Command::HangUp);
    // Lets the leave go out, so the other side sees us go at once instead of after the server's resume grace
    std::thread::sleep(Duration::from_millis(500));
}

/// Counts frames instead of showing them.
#[derive(Default)]
struct LogSink {
    peer_frames: AtomicU32,
}

impl VideoSink for LogSink {
    fn frame(&self, feed: Feed, frame: &Frame<'_>) {
        if feed != Feed::Camera {
            let n = self.peer_frames.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(90) {
                let mut rgba = vec![0; frame.width() as usize * frame.height() as usize * 4];
                frame.write_rgba(&mut rgba);
                tracing::info!("received {feed:?} frame {n}: {}x{}", frame.width(), frame.height());
            }
        }
    }

    fn clear(&self, feed: Feed) {
        tracing::info!("{feed:?} video stopped");
    }
}
