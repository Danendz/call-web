//! Speech recognized on this device with SenseVoice through sherpa-onnx: each utterance the detector cuts out is
//! decoded whole once it ends, on a thread of its own so the mic is never held up. Free, no quota, works offline.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread;
use std::time::Instant;

use protocol::{Lang, TRANSCRIBE_SAMPLE_RATE};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use tokio::sync::mpsc;

use super::model::{SENSE_VOICE, SENSE_VOICE_TOKENS};
use super::{Out, Recognizer, SpeechEvent};

// A phone has fewer big cores and is also encoding video; two decode an utterance in a fraction of its length
const THREADS: i32 = if cfg!(target_os = "android") { 2 } else { 4 };

/// The loaded model, kept between recordings: loading takes over a second on a phone. Live subtitles use the same
/// one. SenseVoice is told the language, so a change of language loads it again.
pub(crate) type Loaded = Arc<Mutex<Option<(Lang, OfflineRecognizer)>>>;

pub(crate) struct Local {
    jobs: std_mpsc::Sender<Out>,
}

impl Local {
    /// Starts the decoding thread, loading the model now so the first utterance doesn't wait for it. Text and a
    /// failed load come back through `events`.
    pub(crate) fn start(
        dir: PathBuf,
        lang: Lang,
        loaded: Loaded,
        events: mpsc::UnboundedSender<SpeechEvent>,
    ) -> Result<Self, String> {
        let (jobs, rx) = std_mpsc::channel();
        thread::Builder::new()
            .name("recognize".into())
            .spawn(move || run(&dir, lang, &loaded, &rx, &events))
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs })
    }
}

impl Recognizer for Local {
    fn send(&self, msg: Out) {
        let _ = self.jobs.send(msg);
    }
}

fn run(
    dir: &Path,
    lang: Lang,
    loaded: &Loaded,
    jobs: &std_mpsc::Receiver<Out>,
    events: &mpsc::UnboundedSender<SpeechEvent>,
) {
    if let Err(e) = load(dir, lang, loaded) {
        tracing::warn!("on-device speech model didn't load: {e}");
        let _ = events.send(SpeechEvent::LocalFailed);
        return;
    }
    let mut pcm: Vec<f32> = Vec::new();
    let mut open = false;
    // Ends when the recording is dropped
    for msg in jobs {
        match msg {
            Out::Start(_) => {
                pcm.clear();
                open = true;
            }
            Out::Audio(block) if open => pcm.extend(block.iter().map(|&s| f32::from(s) / 32768.0)),
            Out::End if open => {
                open = false;
                if let Some(text) = decode(loaded, &pcm) {
                    let _ = events.send(SpeechEvent::Text { text, live_id: None });
                }
            }
            Out::Cancel => open = false,
            _ => {}
        }
    }
}

pub(super) fn load(dir: &Path, lang: Lang, loaded: &Loaded) -> Result<(), String> {
    let mut slot = loaded.lock().map_err(|_| "the model's lock is poisoned".to_owned())?;
    if slot.as_ref().is_some_and(|(l, _)| *l == lang) {
        return Ok(());
    }
    *slot = None;
    let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: path(SENSE_VOICE.local),
        language: Some(
            match lang {
                Lang::Zh => "zh",
                Lang::En => "en",
            }
            .to_owned(),
        ),
        // Numbers and dates as digits, as someone would type them
        use_itn: true,
    };
    config.model_config.tokens = path(SENSE_VOICE_TOKENS.local);
    config.model_config.num_threads = THREADS;
    let started = Instant::now();
    let recognizer = OfflineRecognizer::create(&config).ok_or("sherpa-onnx couldn't create the recognizer")?;
    tracing::info!("on-device speech model loaded in {:?}", started.elapsed());
    *slot = Some((lang, recognizer));
    Ok(())
}

pub(super) fn decode(loaded: &Loaded, pcm: &[f32]) -> Option<String> {
    let slot = loaded.lock().ok()?;
    let (_, recognizer) = slot.as_ref()?;
    let started = Instant::now();
    let stream = recognizer.create_stream();
    stream.accept_waveform(TRANSCRIBE_SAMPLE_RATE as i32, pcm);
    recognizer.decode(&stream);
    let text = stream.get_result()?.text;
    tracing::info!(
        "recognized audio={:.1}s in {:?} chars={}",
        pcm.len() as f32 / TRANSCRIBE_SAMPLE_RATE as f32,
        started.elapsed(),
        text.chars().count()
    );
    clean(&text)
}

/// The line to post, or None when it's only punctuation: a cough or a breath comes out as "。" or nothing.
fn clean(text: &str) -> Option<String> {
    let text = text.trim();
    text.chars().any(char::is_alphanumeric).then(|| text.to_owned())
}
