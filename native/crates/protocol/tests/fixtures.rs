//! Every example in `shared/protocol.fixtures.ts` (written to `shared/fixtures.json` by `npm run fixtures`) must
//! parse into these types and serialize back to the same JSON.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing, reason = "a test fails loudly")]

use protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

const FIXTURES: &str = include_str!("../../../../shared/fixtures.json");

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).unwrap()
}

fn round_trip<T: Serialize + DeserializeOwned>(group: &str) {
    let all = fixtures();
    let examples = all[group].as_array().unwrap_or_else(|| panic!("no {group} in fixtures.json"));
    for example in examples {
        let parsed: T = serde_json::from_value(example.clone()).unwrap_or_else(|e| panic!("{group}: {e}\n{example}"));
        assert_eq!(&serde_json::to_value(&parsed).unwrap(), example, "{group} doesn't round-trip");
    }
}

#[test]
fn client_messages() {
    round_trip::<ClientMessage>("clientMessages");
}

#[test]
fn server_messages() {
    round_trip::<ServerMessage>("serverMessages");
}

#[test]
fn speech_messages() {
    round_trip::<SpeechClientMessage>("speechClientMessages");
    round_trip::<SpeechServerMessage>("speechServerMessages");
}

#[test]
fn user_messages() {
    round_trip::<UserClientMessage>("userClientMessages");
    round_trip::<UserServerMessage>("userServerMessages");
}

#[test]
fn constants() {
    let c = &fixtures()["constants"];
    assert_eq!(c["MAX_CHAT_LENGTH"], MAX_CHAT_LENGTH);
    assert_eq!(c["MAX_STREAM_ID_LENGTH"], MAX_STREAM_ID_LENGTH);
    assert_eq!(c["MAX_LIVE_LENGTH"], MAX_LIVE_LENGTH);
    assert_eq!(c["MAX_LIVE_ID_LENGTH"], MAX_LIVE_ID_LENGTH);
    assert_eq!(c["ROOM_ID_RE"], ROOM_ID_PATTERN, "update is_room_id along with the pattern");
    assert_eq!(c["TRANSCRIBE_SAMPLE_RATE"], TRANSCRIBE_SAMPLE_RATE);
    assert_eq!(c["MAX_UTTERANCE_S"], MAX_UTTERANCE_S);
    assert_eq!(c["LANGS"], serde_json::to_value(Lang::ALL).unwrap());
    assert_eq!(c["REACTIONS"], serde_json::to_value(REACTIONS).unwrap());
    assert_eq!(c["EMOJI"], serde_json::to_value(EMOJI.as_slice()).unwrap());
    for e in REACTIONS.iter().chain(EMOJI.iter()) {
        let r = Reaction::parse(e).unwrap();
        assert_eq!(serde_json::to_value(r).unwrap(), *e);
    }
    assert_eq!(c["USERNAME_RE"], USERNAME_PATTERN, "update is_username along with the pattern");
    assert_eq!(c["MIN_PASSWORD_LENGTH"], MIN_PASSWORD_LENGTH);
    assert_eq!(c["MAX_PASSWORD_LENGTH"], MAX_PASSWORD_LENGTH);
}

#[test]
fn unknown_server_message_is_ignored_not_an_error() {
    let msg: ServerMessage = serde_json::from_str(r#"{"type":"from-the-future","x":1}"#).unwrap();
    assert_eq!(msg, ServerMessage::Unknown);
}

#[test]
fn a_reaction_outside_the_allowlist_does_not_parse() {
    assert!(serde_json::from_str::<ServerMessage>(r#"{"type":"reaction","emoji":"hello"}"#).is_err());
    assert!(Reaction::parse("🦀").is_none());
}
