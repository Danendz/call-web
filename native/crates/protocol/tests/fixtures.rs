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
fn constants() {
    let c = &fixtures()["constants"];
    assert_eq!(c["MAX_CHAT_LENGTH"], MAX_CHAT_LENGTH);
    assert_eq!(c["MAX_STREAM_ID_LENGTH"], MAX_STREAM_ID_LENGTH);
    assert_eq!(c["ROOM_ID_RE"], ROOM_ID_PATTERN, "update is_room_id along with the pattern");
    assert_eq!(c["TRANSCRIBE_SAMPLE_RATE"], TRANSCRIBE_SAMPLE_RATE);
    assert_eq!(c["MAX_UTTERANCE_S"], MAX_UTTERANCE_S);
    assert_eq!(c["LANGS"], serde_json::to_value(Lang::ALL).unwrap());
}

#[test]
fn unknown_server_message_is_ignored_not_an_error() {
    let msg: ServerMessage = serde_json::from_str(r#"{"type":"from-the-future","x":1}"#).unwrap();
    assert_eq!(msg, ServerMessage::Unknown);
}
