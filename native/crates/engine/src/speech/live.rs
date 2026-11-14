//! Speech recognized on this device while it's said, for live subtitles, like the server's `speechLines.ts`: the
//! same SenseVoice as [`super::local`] decodes the line so far again as it grows, and each result goes out as live
//! text. Silero VAD says where the speech is: a short pause in it ends a line, so a long stretch of talk becomes a few
//! short lines that each show and finish quickly, and an utterance with no speech in it posts nothing.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::thread;

use protocol::{Lang, TRANSCRIBE_SAMPLE_RATE};
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use tokio::sync::mpsc;

use super::local::{Loaded, decode, load};
use super::model::SILERO_VAD;
use super::{Out, Recognizer, SpeechEvent};

const RATE: usize = TRANSCRIBE_SAMPLE_RATE as usize;
// Live text is decoded again after this much new audio; less often once the line is long, since every decode is of
// the whole line so far and takes longer
const LIVE_EVERY_S: f32 = 0.5;
const LIVE_EVERY_LONG_S: f32 = 1.0;
const LONG_LINE_S: f32 = 5.0;
// A pause in speech this long ends a line
const PAUSE_S: f32 = 0.4;
// Talk without a pause this long is cut into a line anyway
const MAX_LINE_S: f32 = 20.0;
// What the VAD holds: a whole utterance, at most MAX_UTTERANCE_S, with room to spare
const VAD_BUFFER_S: f32 = 40.0;
// Audio kept from before the VAD says speech started, which it says a little late: without it, the first syllable is
// cut. More than the server keeps: each utterance here starts with the detector's preroll, and the VAD takes up to
// half a second of it to be sure.
const KEEP_BEFORE_SPEECH_S: f32 = 1.0;
// Kept past where the VAD says speech ended: most of the pause it waited for. SenseVoice reads a line better with
// some silence after it, but the next line can start within the pause's last tenth of a second.
const KEEP_AFTER_SPEECH_S: f32 = 0.3;

/// The recognizer for a recording: decodes on a thread of its own so the mic is never held up.
pub(crate) struct Live {
    jobs: std_mpsc::Sender<Out>,
}

impl Live {
    /// Starts the decoding thread, loading SenseVoice now (or using it already loaded) so the first utterance doesn't
    /// wait for it. Text and a failed load come back through `events`.
    pub(crate) fn start(
        dir: PathBuf,
        lang: Lang,
        loaded: Loaded,
        events: mpsc::UnboundedSender<SpeechEvent>,
    ) -> Result<Self, String> {
        let (jobs, rx) = std_mpsc::channel();
        thread::Builder::new()
            .name("recognize live".into())
            .spawn(move || run(&dir, lang, &loaded, &rx, &events))
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs })
    }
}

impl Recognizer for Live {
    fn send(&self, msg: Out) {
        let _ = self.jobs.send(msg);
    }
}

fn vad(dir: &Path) -> Result<VoiceActivityDetector, String> {
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(dir.join(SILERO_VAD.local).to_string_lossy().into_owned()),
            threshold: 0.5,
            min_silence_duration: PAUSE_S,
            min_speech_duration: 0.25,
            window_size: 512,
            max_speech_duration: MAX_LINE_S,
        },
        sample_rate: TRANSCRIBE_SAMPLE_RATE as i32,
        num_threads: 1,
        ..Default::default()
    };
    VoiceActivityDetector::create(&config, VAD_BUFFER_S).ok_or_else(|| "sherpa-onnx couldn't create the VAD".to_owned())
}

fn run(
    dir: &Path,
    lang: Lang,
    loaded: &Loaded,
    jobs: &std_mpsc::Receiver<Out>,
    events: &mpsc::UnboundedSender<SpeechEvent>,
) {
    let vad = match load(dir, lang, loaded).and_then(|()| vad(dir)) {
        Ok(vad) => vad,
        Err(e) => {
            tracing::warn!("on-device live speech didn't load: {e}");
            let _ = events.send(SpeechEvent::LocalFailed);
            return;
        }
    };
    let mut lines = Lines::new(
        vad,
        |pcm: &[f32]| decode(loaded, pcm),
        |event| {
            let _ = events.send(event);
        },
    );
    // Ends when the recording is dropped
    while let Ok(msg) = jobs.recv() {
        lines.handle(msg);
        // What came in meanwhile goes in before the next live decode: when decoding falls behind, there's one decode
        // of everything instead of a stale one per chunk
        while let Ok(msg) = jobs.try_recv() {
            lines.handle(msg);
        }
        lines.decode_live();
    }
}

/// Where speech is, as [`Lines`] needs it: Silero on a device, a script in tests.
trait Vad {
    fn accept(&mut self, samples: &[f32]);
    /// Where speech ended since the last call, in samples since the last reset: a pause ended it, or it ran too
    /// long. The VAD says so only once the pause is long enough, by which time the next line may have started.
    fn ended(&mut self) -> Option<usize>;
    /// Ends the speech still going on, so the next `ended` says so.
    fn flush(&mut self);
    fn speaking(&self) -> bool;
    fn reset(&mut self);
}

impl Vad for VoiceActivityDetector {
    fn accept(&mut self, samples: &[f32]) {
        self.accept_waveform(samples);
    }

    fn ended(&mut self) -> Option<usize> {
        let mut end = None;
        while let Some(segment) = self.front() {
            end = Some(usize::try_from(segment.start()).unwrap_or(0) + segment.samples().len());
            self.pop();
        }
        end
    }

    fn flush(&mut self) {
        VoiceActivityDetector::flush(self);
    }

    fn speaking(&self) -> bool {
        self.detected()
    }

    fn reset(&mut self) {
        VoiceActivityDetector::reset(self);
    }
}

/// One recording's utterances cut into lines, with the live text of the line being said.
struct Lines<V, D, E> {
    vad: V,
    decode: D,
    emit: E,
    open: bool,
    /// The audio since the last line ended
    pcm: Vec<f32>,
    /// Where `pcm` starts, in samples fed to the VAD since its last reset
    pcm_start: usize,
    /// `pcm.len()` when live text was last decoded
    decoded_at: usize,
    live_id: Option<String>,
}

impl<V: Vad, D: Fn(&[f32]) -> Option<String>, E: Fn(SpeechEvent)> Lines<V, D, E> {
    fn new(vad: V, decode: D, emit: E) -> Self {
        Self { vad, decode, emit, open: false, pcm: Vec::new(), pcm_start: 0, decoded_at: 0, live_id: None }
    }

    fn handle(&mut self, msg: Out) {
        match msg {
            Out::Start(_) => {
                self.stop();
                self.vad.reset();
                self.pcm_start = 0;
                self.open = true;
            }
            Out::Audio(block) if self.open => {
                let samples: Vec<f32> = block.iter().map(|&s| f32::from(s) / 32768.0).collect();
                self.vad.accept(&samples);
                self.pcm.extend_from_slice(&samples);
                if let Some(end) = self.vad.ended() {
                    self.finish_line(end);
                }
                if !self.vad.speaking() {
                    let keep = (KEEP_BEFORE_SPEECH_S * RATE as f32) as usize;
                    let extra = self.pcm.len().saturating_sub(keep);
                    self.pcm.drain(..extra);
                    self.pcm_start += extra;
                    self.decoded_at = 0;
                }
            }
            Out::End if self.open => {
                self.vad.flush();
                if self.vad.ended().is_some() {
                    // Nothing more is coming: it all goes in this line
                    self.finish_line(usize::MAX);
                }
                self.stop();
            }
            Out::Cancel => self.stop(),
            _ => {}
        }
    }

    /// Decodes the line so far again, once enough new audio came in since the last time.
    fn decode_live(&mut self) {
        if !self.open || !self.vad.speaking() {
            return;
        }
        let every =
            if self.pcm.len() > (LONG_LINE_S * RATE as f32) as usize { LIVE_EVERY_LONG_S } else { LIVE_EVERY_S };
        if self.pcm.len().saturating_sub(self.decoded_at) < (every * RATE as f32) as usize {
            return;
        }
        self.decoded_at = self.pcm.len();
        let Some(text) = (self.decode)(&self.pcm).and_then(|t| clean(&t)) else { return };
        let id = self.live_id.get_or_insert_with(crate::room::random_id).clone();
        (self.emit)(SpeechEvent::Live { id, text });
    }

    /// Ends the line where speech ended; what came after it starts the next one.
    fn finish_line(&mut self, end: usize) {
        let live_id = self.live_id.take();
        let after = (KEEP_AFTER_SPEECH_S * RATE as f32) as usize;
        let cut = end.saturating_sub(self.pcm_start).saturating_add(after).min(self.pcm.len());
        let rest = self.pcm.split_off(cut);
        let pcm = std::mem::replace(&mut self.pcm, rest);
        self.pcm_start += cut;
        self.decoded_at = 0;
        match (self.decode)(&pcm).and_then(|t| clean(&t)) {
            Some(text) => (self.emit)(SpeechEvent::Text { text, live_id }),
            // Nothing to post: its live text is taken back
            None => {
                if let Some(id) = live_id {
                    (self.emit)(SpeechEvent::Live { id, text: String::new() });
                }
            }
        }
    }

    /// The utterance is over; live text that never became a line is taken back.
    fn stop(&mut self) {
        self.open = false;
        self.pcm.clear();
        self.decoded_at = 0;
        if let Some(id) = self.live_id.take() {
            (self.emit)(SpeechEvent::Live { id, text: String::new() });
        }
    }
}

/// The line to post, or None when it's only punctuation or a filler: in the chat a lone "嗯" is noise even when it
/// was really said, since it was heard anyway.
fn clean(text: &str) -> Option<String> {
    let words: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    (!words.is_empty() && !fillers(&words)).then(|| text.trim().to_owned())
}

/// Whether `words` is fillers and nothing else, like the server's `FILLER_RE`.
fn fillers(words: &[char]) -> bool {
    let Some((first, rest)) = words.split_first() else { return true };
    if "嗯啊哦呃额唔".contains(*first) && fillers(rest) {
        return true;
    }
    for fixed in [['h', 'u', 'h'], ['m', 'h', 'm']] {
        if let Some(rest) = words.strip_prefix(&fixed[..])
            && fillers(rest)
        {
            return true;
        }
    }
    // "mm", "hmm", "uhh", "umm", "ahh", "ohh": a letter, then one or more of another
    for (letter, repeated) in [('m', 'm'), ('h', 'm'), ('u', 'h'), ('u', 'm'), ('a', 'h'), ('o', 'h')] {
        if *first != letter {
            continue;
        }
        let run = rest.iter().take_while(|&&c| c == repeated).count();
        if (1..=run).any(|n| rest.get(n..).is_some_and(fillers)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use protocol::Lang;

    use super::{Lines, Out, RATE, SpeechEvent, Vad, clean};

    /// What the VAD says, as the test sets it
    #[derive(Default)]
    struct Script {
        speaking: bool,
        /// Where speech ended, in samples
        ended: Option<usize>,
        /// Speech still going on, which a flush ends
        open: bool,
    }

    #[derive(Clone, Default)]
    struct FakeVad(Rc<RefCell<Script>>);

    impl Vad for FakeVad {
        fn accept(&mut self, _: &[f32]) {}
        fn ended(&mut self) -> Option<usize> {
            self.0.borrow_mut().ended.take()
        }
        fn flush(&mut self) {
            let mut s = self.0.borrow_mut();
            if std::mem::take(&mut s.open) {
                s.ended = Some(usize::MAX);
            }
        }
        fn speaking(&self) -> bool {
            self.0.borrow().speaking
        }
        fn reset(&mut self) {}
    }

    fn audio(seconds: f32) -> Out {
        Out::Audio(vec![0; (seconds * RATE as f32) as usize])
    }

    type Log = Rc<RefCell<Vec<String>>>;
    type TestLines = Lines<FakeVad, Box<dyn Fn(&[f32]) -> Option<String>>, Box<dyn Fn(SpeechEvent)>>;

    /// Decodes to "<seconds>s", or to `said`; what goes out is logged
    fn lines(said: Option<&'static str>) -> (FakeVad, Log, TestLines) {
        let vad = FakeVad::default();
        let log = Log::default();
        let out = log.clone();
        let lines: TestLines = Lines::new(
            vad.clone(),
            Box::new(move |pcm: &[f32]| {
                Some(said.map_or_else(|| format!("{}s", pcm.len() as f32 / RATE as f32), str::to_owned))
            }),
            Box::new(move |event: SpeechEvent| {
                out.borrow_mut().push(match event {
                    SpeechEvent::Live { text, .. } => format!("live {text}"),
                    SpeechEvent::Text { text, live_id } => format!("line {text} replaces={}", live_id.is_some()),
                    _ => String::new(),
                });
            }),
        );
        (vad, log, lines)
    }

    #[test]
    fn a_pause_ends_a_line_with_the_live_text_it_replaces() {
        let (vad, log, mut lines) = lines(None);
        lines.handle(Out::Start(Lang::En));
        vad.0.borrow_mut().speaking = true;
        lines.handle(audio(0.5));
        lines.decode_live();
        lines.handle(audio(0.2));
        lines.decode_live(); // not 0.5 s of new audio yet
        vad.0.borrow_mut().ended = Some(usize::MAX);
        lines.handle(audio(0.1));
        // The next line, still being said when the utterance ends
        lines.handle(audio(0.3));
        vad.0.borrow_mut().open = true;
        lines.handle(Out::End);
        assert_eq!(*log.borrow(), ["live 0.5s", "line 0.8s replaces=true", "line 0.3s replaces=false"]);
    }

    #[test]
    fn a_line_ends_where_speech_did_and_the_next_one_keeps_its_start() {
        let (vad, log, mut lines) = lines(None);
        lines.handle(Out::Start(Lang::En));
        vad.0.borrow_mut().speaking = true;
        lines.handle(audio(1.0));
        // The VAD sees the pause only once the next line has begun: speech ended 0.6 s in
        vad.0.borrow_mut().ended = Some(RATE * 6 / 10);
        lines.handle(audio(0.2));
        vad.0.borrow_mut().open = true;
        lines.handle(Out::End);
        // The first line keeps 0.3 s past its end; the other 0.3 s starts the next
        assert_eq!(*log.borrow(), ["line 0.9s replaces=false", "line 0.3s replaces=false"]);
    }

    #[test]
    fn quiet_audio_is_trimmed_and_live_text_without_a_line_is_taken_back() {
        let (vad, log, mut lines) = lines(None);
        lines.handle(Out::Start(Lang::En));
        // Ten seconds with no speech keeps only the last second
        for _ in 0..20 {
            lines.handle(audio(0.5));
        }
        vad.0.borrow_mut().speaking = true;
        lines.handle(audio(0.5));
        lines.decode_live();
        lines.handle(Out::Cancel);
        assert_eq!(*log.borrow(), ["live 1.5s", "live "]);
    }

    #[test]
    fn a_filler_alone_isnt_posted() {
        let (vad, log, mut lines) = lines(Some("嗯。"));
        lines.handle(Out::Start(Lang::Zh));
        vad.0.borrow_mut().open = true;
        lines.handle(audio(0.5));
        lines.handle(Out::End);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn fillers_and_words() {
        for filler in ["嗯。", "Mm-hmm.", "Uhh, um", "Hmm?", "啊哦", "mhm"] {
            assert_eq!(clean(filler), None, "{filler}");
        }
        for words in ["嗯，好的。", "Hmm, maybe.", "Oh no", "um, yes", "Ahead"] {
            assert_eq!(clean(words).as_deref(), Some(words), "{words}");
        }
    }
}
