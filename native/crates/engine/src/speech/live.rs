//! Speech recognized on this device while it's said, for live subtitles: sherpa-onnx's streaming Zipformer
//! (Mandarin and English, mixed) decodes each utterance as its audio comes, so its text so far goes out as live text
//! a few times a second. Once the utterance ends, a CT-Transformer punctuates it into the chat line.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread;
use std::time::{Duration, Instant};

use protocol::TRANSCRIBE_SAMPLE_RATE;
use sherpa_onnx::{
    OfflinePunctuation, OfflinePunctuationConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
};
use tokio::sync::mpsc;

use super::model::{DECODER, ENCODER, JOINER, LIVE_TOKENS, PUNCTUATION};
use super::{Out, Recognizer, SpeechEvent};

// Streaming decodes 40 ms at a time, far lighter than SenseVoice's whole utterance
const THREADS: i32 = 2;
// Live text goes out at most this often, under the server's limit of 8 a second
const LIVE_EVERY: Duration = Duration::from_millis(250);

/// The loaded models, kept between recordings: loading takes a second or so. A recording holds its own handle, so
/// dropping this frees them once it stops.
pub(crate) type LoadedLive = Arc<Mutex<Option<Arc<LiveModel>>>>;

/// The streaming recognizer and the punctuation model.
pub struct LiveModel {
    recognizer: OnlineRecognizer,
    punctuation: OfflinePunctuation,
}

impl LiveModel {
    /// Loads the models from their folder in the app's data folder.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = path(ENCODER.local);
        config.model_config.transducer.decoder = path(DECODER.local);
        config.model_config.transducer.joiner = path(JOINER.local);
        config.model_config.tokens = path(LIVE_TOKENS.local);
        config.model_config.num_threads = THREADS;
        config.decoding_method = Some("greedy_search".into());
        let recognizer =
            OnlineRecognizer::create(&config).ok_or("sherpa-onnx couldn't create the streaming recognizer")?;
        let mut config = OfflinePunctuationConfig::default();
        config.model.ct_transformer = path(PUNCTUATION.local);
        config.model.num_threads = 1;
        let punctuation = OfflinePunctuation::create(&config).ok_or("sherpa-onnx couldn't create the punctuation")?;
        Ok(Self { recognizer, punctuation })
    }

    /// Starts decoding an utterance.
    pub fn start(self: &Arc<Self>) -> Utterance {
        Utterance { model: self.clone(), stream: self.recognizer.create_stream(), text: String::new() }
    }
}

/// One utterance being decoded.
pub struct Utterance {
    model: Arc<LiveModel>,
    stream: OnlineStream,
    text: String,
}

impl Utterance {
    /// Decodes 16 kHz audio in -1..1; returns the text so far when it changed.
    pub fn push(&mut self, pcm: &[f32]) -> Option<&str> {
        let recognizer = &self.model.recognizer;
        self.stream.accept_waveform(TRANSCRIBE_SAMPLE_RATE as i32, pcm);
        while recognizer.is_ready(&self.stream) {
            recognizer.decode(&self.stream);
        }
        let text = tidy(&recognizer.get_result(&self.stream)?.text);
        (text != self.text).then(|| {
            self.text = text;
            self.text.as_str()
        })
    }

    /// Decodes what's left and punctuates it; None when nothing was said.
    pub fn finish(mut self) -> Option<String> {
        // The model looks a little ahead, so the last words only come out after some silence
        self.push(&[0.0; TRANSCRIBE_SAMPLE_RATE as usize / 2]);
        self.stream.input_finished();
        self.push(&[]);
        if !self.text.chars().any(char::is_alphanumeric) {
            return None;
        }
        let punctuated = self.model.punctuation.add_punctuation(&self.text).unwrap_or_else(|| self.text.clone());
        Some(capitalize(&punctuated))
    }
}

/// The recognizer's text as someone would write it: its English comes in capitals, with spaces only between words.
fn tidy(text: &str) -> String {
    let lower = text.trim().to_lowercase();
    capitalize(&lower)
}

/// English starts its sentences, and "I", with a capital; Chinese has none, so it's left as is.
fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut sentence_start = true;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let after = chars.get(i + 1).copied();
        // A lone "i", or "i" in "i'm", "i'll"
        let lone_i = c == 'i'
            && !before.is_some_and(|b| b.is_ascii_alphabetic())
            && !after.is_some_and(|a| a.is_ascii_alphabetic());
        if c.is_ascii_lowercase() && (sentence_start || lone_i) {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
        if matches!(c, '.' | '?' | '!' | '。' | '？' | '！') {
            sentence_start = true;
        } else if !c.is_whitespace() {
            sentence_start = false;
        }
    }
    out
}

/// The recognizer for a recording: decodes on a thread of its own so the mic is never held up.
pub(crate) struct Live {
    jobs: std_mpsc::Sender<Out>,
}

impl Live {
    /// Starts the decoding thread, loading the models now so the first utterance doesn't wait for them. Text and a
    /// failed load come back through `events`.
    pub(crate) fn start(
        dir: PathBuf,
        loaded: LoadedLive,
        events: mpsc::UnboundedSender<SpeechEvent>,
    ) -> Result<Self, String> {
        let (jobs, rx) = std_mpsc::channel();
        thread::Builder::new()
            .name("recognize live".into())
            .spawn(move || run(&dir, &loaded, &rx, &events))
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs })
    }
}

impl Recognizer for Live {
    fn send(&self, msg: Out) {
        let _ = self.jobs.send(msg);
    }
}

fn load(dir: &Path, loaded: &LoadedLive) -> Result<Arc<LiveModel>, String> {
    let mut slot = loaded.lock().map_err(|_| "the model's lock is poisoned".to_owned())?;
    if let Some(model) = slot.as_ref() {
        return Ok(model.clone());
    }
    let started = Instant::now();
    let model = Arc::new(LiveModel::load(dir)?);
    tracing::info!("live speech model loaded in {:?}", started.elapsed());
    *slot = Some(model.clone());
    Ok(model)
}

/// The utterance being said: its live id, its decoding, and when its text last went out.
struct Open {
    id: String,
    utterance: Utterance,
    sent: Option<Instant>,
    /// Text newer than what went out
    unsent: Option<String>,
}

fn run(dir: &Path, loaded: &LoadedLive, jobs: &std_mpsc::Receiver<Out>, events: &mpsc::UnboundedSender<SpeechEvent>) {
    let model = match load(dir, loaded) {
        Ok(model) => model,
        Err(e) => {
            tracing::warn!("live speech model didn't load: {e}");
            let _ = events.send(SpeechEvent::LocalFailed);
            return;
        }
    };
    let mut open: Option<Open> = None;
    // Ends when the recording is dropped
    for msg in jobs {
        match msg {
            Out::Start(_) => {
                open = Some(Open { id: crate::room::random_id(), utterance: model.start(), sent: None, unsent: None });
            }
            Out::Audio(block) => {
                let Some(o) = open.as_mut() else { continue };
                let pcm: Vec<f32> = block.iter().map(|&s| f32::from(s) / 32768.0).collect();
                if let Some(text) = o.utterance.push(&pcm) {
                    o.unsent = Some(text.to_owned());
                }
                if o.sent.is_none_or(|at| at.elapsed() >= LIVE_EVERY)
                    && let Some(text) = o.unsent.take()
                {
                    o.sent = Some(Instant::now());
                    let _ = events.send(SpeechEvent::Live { id: o.id.clone(), text });
                }
            }
            Out::End => {
                let Some(o) = open.take() else { continue };
                let started = Instant::now();
                match o.utterance.finish() {
                    Some(text) => {
                        tracing::info!(
                            "finished a live line in {:?} chars={}",
                            started.elapsed(),
                            text.chars().count()
                        );
                        let _ = events.send(SpeechEvent::Text { text, live_id: o.sent.map(|_| o.id) });
                    }
                    None => cancel(o.id, o.sent, events),
                }
            }
            Out::Cancel => {
                if let Some(o) = open.take() {
                    cancel(o.id, o.sent, events);
                }
            }
        }
    }
}

/// Drops the live line, if any of it went out
fn cancel(id: String, sent: Option<Instant>, events: &mpsc::UnboundedSender<SpeechEvent>) {
    if sent.is_some() {
        let _ = events.send(SpeechEvent::Live { id, text: String::new() });
    }
}

#[cfg(test)]
mod tests {
    use super::{capitalize, tidy};

    #[test]
    fn english_comes_out_written() {
        assert_eq!(tidy("HELLO I'M HERE AND I THINK"), "Hello I'm here and I think");
        assert_eq!(tidy("我们 GO TO THE 公园"), "我们 go to the 公园");
    }

    #[test]
    fn sentences_start_with_a_capital() {
        assert_eq!(capitalize("ok. so it is? yes, i see"), "Ok. So it is? Yes, I see");
        assert_eq!(capitalize("好的。then we go"), "好的。Then we go");
        // Not "I" inside words
        assert_eq!(capitalize("in it is fine"), "In it is fine");
    }
}
