//! Joins a room without the UI and logs every state change, for checking calls when the app can't be driven (a
//! locked screen, CI) or as the second person in a native ↔ native or native ↔ browser test:
//!
//!   cargo run -p engine --example join -- http://localhost:8090 <room id or link> [seconds]

// A dev tool: bad arguments just stop it
#![allow(clippy::panic)]

use std::time::Duration;

use engine::{Command, Engine, Lang, room};
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

    let engine = Engine::start(server, Lang::En, |state| {
        tracing::info!(
            "phase={:?} link={:?} reconnecting={} in_call_for={:?} mics={}",
            state.phase,
            state.link,
            state.reconnecting,
            state.call_started.map(|at| at.elapsed().as_secs()),
            state.mics.len()
        );
    })
    .unwrap_or_else(|e| panic!("engine: {e}"));
    engine.send(Command::OpenRoom(room));
    engine.send(Command::Join);
    std::thread::sleep(Duration::from_secs(seconds));
    engine.send(Command::HangUp);
    // Lets the leave go out, so the other side sees us go at once instead of after the server's resume grace
    std::thread::sleep(Duration::from_millis(500));
}
