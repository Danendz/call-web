//! Cuts mic audio into utterances, like the web's `whisperRecorder.ts`: loudness against the room's noise floor says
//! when someone is talking, a pause ends it. Pure logic over 16 kHz chunks, so the thresholds can be tested.

use protocol::MAX_UTTERANCE_S;

/// 40 ms at 16 kHz; close to the web's 2048-sample chunks at 48 kHz.
pub(crate) const CHUNK: usize = 640;
const CHUNK_MS: u32 = 40;

// Louder than the noise floor by this much is speech, and never quieter than MIN_SPEECH_DB
const ABOVE_FLOOR_DB: f32 = 12.0;
const MIN_SPEECH_DB: f32 = -50.0;
// Loud chunks in a row that start an utterance (~80 ms), so a click doesn't
const START_CHUNKS: u32 = 2;
// Kept from before the start, so the first syllable isn't cut
const PREROLL_MS: u32 = 300;
const END_SILENCE_MS: u32 = 800;
// Shorter than this of actual speech is a cough or a bump, not worth a request
const MIN_VOICED_MS: u32 = 300;

#[derive(Debug, PartialEq)]
pub(crate) enum Event {
    Start,
    Audio(Vec<i16>),
    /// Whether it was speech worth transcribing; false cancels it
    End(bool),
}

pub(crate) struct Detector {
    floor_db: f32,
    loud_run: u32,
    preroll: Vec<Vec<i16>>,
    in_utterance: bool,
    utterance_ms: u32,
    voiced_ms: u32,
    silent_ms: u32,
}

impl Detector {
    pub(crate) fn new() -> Self {
        Self {
            floor_db: -60.0,
            loud_run: 0,
            preroll: Vec::new(),
            in_utterance: false,
            utterance_ms: 0,
            voiced_ms: 0,
            silent_ms: 0,
        }
    }

    /// Feeds one [`CHUNK`]; what it starts, carries or ends comes back in order.
    pub(crate) fn push(&mut self, chunk: Vec<i16>, out: &mut Vec<Event>) {
        let db = level_db(&chunk);
        let loud = db > (self.floor_db + ABOVE_FLOOR_DB).max(MIN_SPEECH_DB);

        if !self.in_utterance {
            // Drops to quiet at once, creeps up to steady noise slowly
            if !loud {
                self.floor_db = if db < self.floor_db { db } else { self.floor_db + (db - self.floor_db) * 0.05 };
            }
            self.preroll.push(chunk);
            if self.preroll.len() as u32 * CHUNK_MS > PREROLL_MS {
                self.preroll.remove(0);
            }
            self.loud_run = if loud { self.loud_run + 1 } else { 0 };
            if self.loud_run >= START_CHUNKS {
                self.in_utterance = true;
                self.utterance_ms = self.preroll.len() as u32 * CHUNK_MS;
                self.voiced_ms = self.loud_run * CHUNK_MS;
                self.silent_ms = 0;
                out.push(Event::Start);
                out.extend(self.preroll.drain(..).map(Event::Audio));
            }
            return;
        }

        out.push(Event::Audio(chunk));
        self.utterance_ms += CHUNK_MS;
        if loud {
            self.voiced_ms += CHUNK_MS;
            self.silent_ms = 0;
        } else {
            self.silent_ms += CHUNK_MS;
        }
        if self.silent_ms >= END_SILENCE_MS || self.utterance_ms >= MAX_UTTERANCE_S * 1000 - 500 {
            self.in_utterance = false;
            self.loud_run = 0;
            out.push(Event::End(self.voiced_ms >= MIN_VOICED_MS));
        }
    }

    /// Cut off mid-sentence (muted, language changed): what was said so far is dropped.
    pub(crate) fn stop(&mut self, out: &mut Vec<Event>) {
        if self.in_utterance {
            out.push(Event::End(false));
        }
        *self = Self::new();
    }
}

fn level_db(chunk: &[i16]) -> f32 {
    let sum: f32 = chunk.iter().map(|&s| (f32::from(s) / 32768.0).powi(2)).sum();
    10.0 * (sum / chunk.len().max(1) as f32 + 1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: i16) -> Vec<i16> {
        (0..CHUNK).map(|i| if i % 2 == 0 { amplitude } else { -amplitude }).collect()
    }

    fn feed(d: &mut Detector, chunk: Vec<i16>, n: usize) -> Vec<Event> {
        let mut out = Vec::new();
        for _ in 0..n {
            d.push(chunk.clone(), &mut out);
        }
        out
    }

    fn summary(events: &[Event]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                Event::Start => "start",
                Event::Audio(_) => "audio",
                Event::End(true) => "end",
                Event::End(false) => "cancel",
            })
            .collect()
    }

    #[test]
    fn speech_after_quiet_is_sent_with_preroll_and_ends_on_a_pause() {
        let mut d = Detector::new();
        assert!(feed(&mut d, tone(30), 20).is_empty(), "room noise starts nothing");
        let start = feed(&mut d, tone(8000), 2);
        // Start, then the 300 ms kept from before it (7 chunks of 40 ms, the last two loud)
        assert_eq!(summary(&start).first(), Some(&"start"));
        assert_eq!(start.len(), 1 + 7);
        feed(&mut d, tone(8000), 10);
        let end = feed(&mut d, tone(30), 20);
        assert_eq!(summary(&end).iter().filter(|&&e| e == "end").count(), 1);
        assert_eq!(end.len(), 21, "the 800 ms of pause is still sent, then it ends");
    }

    #[test]
    fn a_short_bump_is_cancelled_not_transcribed() {
        let mut d = Detector::new();
        feed(&mut d, tone(30), 20);
        feed(&mut d, tone(8000), 2);
        let end = feed(&mut d, tone(30), 20);
        assert_eq!(summary(&end).last(), Some(&"cancel"));
    }

    #[test]
    fn stopping_mid_sentence_cancels_it() {
        let mut d = Detector::new();
        feed(&mut d, tone(30), 20);
        feed(&mut d, tone(8000), 5);
        let mut out = Vec::new();
        d.stop(&mut out);
        assert_eq!(out, vec![Event::End(false)]);
    }
}
