//! Room ids and the links people share them by. A link is the web address (`https://call.danendz.com/r/<id>`),
//! so the same link opens the room in a browser or in this app.

use protocol::is_room_id;
use url::Url;

// The web client's alphabet: no 0/O, 1/l/I, so a link read aloud or retyped survives
const ROOM_ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const ROOM_ID_LEN: usize = 10;

#[expect(clippy::expect_used, reason = "the OS random source failing is not something a call can work around")]
pub fn new_room_id() -> String {
    let mut bytes = [0u8; ROOM_ID_LEN];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes
        .iter()
        .filter_map(|b| ROOM_ALPHABET.get(usize::from(*b) % ROOM_ALPHABET.len()))
        .map(|&c| char::from(c))
        .collect()
}

/// The room in what someone pasted: a full link (any host, so a link from a dev server works too), a link
/// without `https://`, or a bare room id.
pub fn parse_room(input: &str) -> Option<String> {
    let input = input.trim();
    if is_room_id(input) {
        return Some(input.to_owned());
    }
    // "localhost:8090/r/…" parses with "localhost" as its scheme, so anything not http(s) gets one added
    let url = match Url::parse(input) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        _ => Url::parse(&format!("https://{input}")).ok()?,
    };
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    match (segments.next(), segments.next(), segments.next()) {
        (Some("r"), Some(id), None) if is_room_id(id) => Some(id.to_owned()),
        _ => None,
    }
}

pub fn room_url(server: &Url, room_id: &str) -> String {
    let mut url = server.clone();
    url.set_path(&format!("/r/{room_id}"));
    url.set_query(None);
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_people_paste() {
        let id = Some("abcDEF2345".to_owned());
        assert_eq!(parse_room("https://call.danendz.com/r/abcDEF2345"), id);
        assert_eq!(parse_room("  https://call.danendz.com/r/abcDEF2345/  "), id);
        assert_eq!(parse_room("call.danendz.com/r/abcDEF2345"), id);
        assert_eq!(parse_room("http://localhost:5173/r/abcDEF2345?stt=server"), id);
        assert_eq!(parse_room("localhost:8090/r/abcDEF2345"), id);
        assert_eq!(parse_room("abcDEF2345"), id);
    }

    #[test]
    fn rejects_what_is_not_a_room() {
        assert_eq!(parse_room(""), None);
        assert_eq!(parse_room("https://call.danendz.com/"), None);
        assert_eq!(parse_room("https://call.danendz.com/r/abc"), None);
        assert_eq!(parse_room("https://call.danendz.com/r/abcDEF2345/extra"), None);
        assert_eq!(parse_room("https://call.danendz.com/x/abcDEF2345"), None);
        assert_eq!(parse_room("hello there"), None);
    }

    #[test]
    fn new_ids_are_valid_rooms() {
        assert!(is_room_id(&new_room_id()));
    }
}
