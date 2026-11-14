//! Speech to chat: what you say becomes chat messages. The mic comes from the call's capture after WebRTC's echo
//! cancellation, noise suppression and gain control, as the web transcribes the track it sends, so the other
//! person's voice from your speakers isn't transcribed as yours. Then it's cut into utterances, which go to a
//! [`Recognizer`]: with live subtitles, the streaming model on this device, which sends what you're saying as you say
//! it; otherwise SenseVoice on this device once it's downloaded and picked, or the server's `/ws/speech`, streamed
//! while you speak like the web's server path.

mod live;
mod local;
mod model;
mod socket;
mod utterance;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{Lang, SpeechServerMessage, TRANSCRIBE_SAMPLE_RATE};
use tokio::sync::mpsc;
use url::Url;

pub use crate::download::ModelStatus;
use crate::media::{MicFrame, MicTap};
pub use live::{LiveModel, Utterance};
pub use model::Model;
pub(crate) use model::{
    delete as delete_model, dir as model_dir, download as download_model, drop_old_decoder, is_ready as model_ready,
};

use live::{Live, LoadedLive};
use local::{Loaded, Local};
use socket::SpeechSocket;
use utterance::{CHUNK, Detector, Event};

// Opened apart from WebRTC, Android hands apps the mic at its recognition level with no gain: talking at arm's length
// comes in near -55 dB, under the detector's floor and down to a few bits once it's 16-bit. Desktop mics come in
// some 20 dB louder. The call's capture has gain control instead.
const MIC_GAIN: f32 = if cfg!(target_os = "android") { 10.0 } else { 1.0 };

/// Something the person should know about speech to chat; the UI words it in their language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechNotice {
    /// The server has no speech model
    Unavailable,
    /// Out of quota until this epoch ms
    Paused { until_ms: u64 },
}

/// What comes back from recognition, for the engine's thread.
pub(crate) enum SpeechEvent {
    Server(SpeechServerMessage),
    /// A line recognized on this device, to post as spoken; `live_id` when its live text went out
    Text {
        text: String,
        live_id: Option<String>,
    },
    /// What's being said so far, to send as live text; empty drops it
    Live {
        id: String,
        text: String,
    },
    /// The on-device model didn't load; speech goes to the server instead
    LocalFailed,
}

/// One utterance as the detector cuts it out: its start, its audio while it's said, then its end or cancel.
#[derive(Debug)]
pub(crate) enum Out {
    Start(Lang),
    Audio(Vec<i16>),
    End,
    Cancel,
}

/// Where utterances go: the server's `/ws/speech` or the model on this device. Sending never blocks the mic.
pub(crate) trait Recognizer: Send {
    fn send(&self, msg: Out);
}

/// What a recording runs for; a change restarts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Params {
    /// `/ws/speech?room=&token=`
    pub url: Url,
    pub lang: Lang,
    /// The mic's name, as for the level meter
    pub mic: Option<String>,
    /// The model on this device to use, when there is one
    pub local: Option<OnDevice>,
}

/// Which model on this device recognizes speech, and its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OnDevice {
    SenseVoice(PathBuf),
    Live(PathBuf),
}

pub(crate) struct Transcriber {
    /// None without WebRTC's platform audio: the mic is opened apart then
    tap: Option<MicTap>,
    running: Option<(Params, Recording)>,
    events: mpsc::UnboundedSender<SpeechEvent>,
    paused_until: Arc<AtomicU64>,
    unavailable: bool,
    loaded: Loaded,
    loaded_live: LoadedLive,
    /// The model didn't load this launch: the server it is
    local_failed: bool,
}

struct Recording {
    stop: std_mpsc::Sender<()>,
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

impl Transcriber {
    /// What recognition reports comes back through `events`, for the engine's thread.
    pub(crate) fn new(events: mpsc::UnboundedSender<SpeechEvent>, tap: Option<MicTap>) -> Self {
        Self {
            tap,
            running: None,
            events,
            paused_until: Arc::default(),
            unavailable: false,
            loaded: Loaded::default(),
            loaded_live: LoadedLive::default(),
            local_failed: false,
        }
    }

    /// Records for `params`, or stops when None. Cheap to call on every change: it only restarts when they differ.
    pub(crate) fn sync(&mut self, params: Option<Params>) {
        let params = params
            .map(|p| if self.local_failed { Params { local: None, ..p } } else { p })
            .filter(|p| p.local.is_some() || !self.unavailable);
        if self.running.as_ref().map(|(p, _)| p) == params.as_ref() {
            return;
        }
        self.running = None;
        let Some(params) = params else { return };
        let (recognizer, paused_until): (Box<dyn Recognizer>, _) = match &params.local {
            Some(on_device) => {
                let started: Result<Box<dyn Recognizer>, String> = match on_device {
                    OnDevice::SenseVoice(dir) => {
                        Local::start(dir.clone(), params.lang, self.loaded.clone(), self.events.clone())
                            .map(|r| Box::new(r) as Box<dyn Recognizer>)
                    }
                    OnDevice::Live(dir) => Live::start(dir.clone(), self.loaded_live.clone(), self.events.clone())
                        .map(|r| Box::new(r) as Box<dyn Recognizer>),
                };
                match started {
                    // The server's quota doesn't apply here
                    Ok(recognizer) => (recognizer, Arc::default()),
                    Err(e) => {
                        tracing::warn!("on-device speech didn't start: {e}");
                        return;
                    }
                }
            }
            None => {
                (Box::new(SpeechSocket::connect(params.url.clone(), self.events.clone())), self.paused_until.clone())
            }
        };
        match start(&params, recognizer, self.tap.as_ref(), paused_until) {
            Ok(recording) => self.running = Some((params, recording)),
            Err(e) => tracing::info!("speech to chat can't open the mic: {e}"),
        }
    }

    /// Whether utterances go to the model on this device right now.
    pub(crate) fn is_local(&self) -> bool {
        self.running.as_ref().is_some_and(|(p, _)| p.local.is_some())
    }

    /// The model didn't load: from now on speech goes to the server.
    pub(crate) fn local_failed(&mut self) {
        self.local_failed = true;
        self.running = None;
    }

    /// Frees the loaded models (a few hundred MB), once the call is over or a model is gone.
    /// A recording still running holds the live models; its thread gives them back when it ends.
    pub(crate) fn release(&mut self) {
        // Each is dropped by the end of its statement, before giving back
        let local = self.loaded.lock().ok().and_then(|mut l| l.take()).is_some();
        let live = self.loaded_live.lock().ok().and_then(|mut l| l.take()).and_then(Arc::into_inner).is_some();
        if local || live {
            crate::memory::give_back();
        }
    }

    /// A notice from the server; returns what the UI should show, if it changed.
    pub(crate) fn notice(&mut self, msg: SpeechServerMessage) -> Option<SpeechNotice> {
        match msg {
            SpeechServerMessage::Unavailable => {
                self.unavailable = true;
                if !self.is_local() {
                    self.running = None;
                }
                Some(SpeechNotice::Unavailable)
            }
            SpeechServerMessage::Paused { until } => {
                self.paused_until.store(until, Ordering::Relaxed);
                Some(SpeechNotice::Paused { until_ms: until })
            }
            _ => None,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where a recording's samples come from.
enum Source {
    /// The call's capture
    Call(std_mpsc::Receiver<MicFrame>),
    /// The mic opened apart, or `CALL_FAKE_MIC`; the Input keeps it open
    Apart(crate::mic::Input, std_mpsc::Receiver<Vec<f32>>),
}

impl Source {
    fn open(tap: Option<&MicTap>, mic: Option<&str>) -> Result<Self, String> {
        if let Some(tap) = tap
            && std::env::var_os("CALL_FAKE_MIC").is_none()
        {
            return Ok(Self::Call(tap.open()));
        }
        let (samples_tx, samples) = std_mpsc::channel();
        let input = crate::mic::open(mic, move |s| {
            let _ = samples_tx.send(s.iter().map(|x| x * MIC_GAIN).collect());
        })?;
        Ok(Self::Apart(input, samples))
    }

    /// The next samples in -1..1 and their rate, or None after `timeout` without any.
    fn recv(&self, timeout: std::time::Duration) -> Option<(Vec<f32>, u32)> {
        match self {
            Self::Call(frames) => {
                // WebRTC's floats are in the 16-bit range
                let (frame, rate) = frames.recv_timeout(timeout).ok()?;
                Some((frame.iter().map(|x| x / 32768.0).collect(), rate))
            }
            Self::Apart(input, samples) => Some((samples.recv_timeout(timeout).ok()?, input.sample_rate)),
        }
    }
}

/// Opens the mic on a thread of its own and runs samples through the pipeline until the Recording is dropped.
fn start(
    params: &Params,
    recognizer: Box<dyn Recognizer>,
    tap: Option<&MicTap>,
    paused_until: Arc<AtomicU64>,
) -> Result<Recording, String> {
    let (stop, stopped) = std_mpsc::channel::<()>();
    let (opened_tx, opened) = std_mpsc::channel();
    let (mic, lang, tap) = (params.mic.clone(), params.lang, tap.cloned());
    thread::Builder::new()
        .name("speech".into())
        .spawn(move || {
            let source = match Source::open(tap.as_ref(), mic.as_deref()) {
                Ok(source) => source,
                Err(e) => {
                    let _ = opened_tx.send(Err(e));
                    return;
                }
            };
            let _ = opened_tx.send(Ok(()));
            let mut down: Option<Downsampler> = None;
            let mut detector = Detector::new();
            let mut pcm: Vec<i16> = Vec::new();
            let mut events = Vec::new();
            // Out of quota when an utterance starts: none of it goes out
            let mut sending = false;
            loop {
                // Told to stop, or the Recording is gone
                if !matches!(stopped.try_recv(), Err(std_mpsc::TryRecvError::Empty)) {
                    break;
                }
                let Some((chunk, rate)) = source.recv(std::time::Duration::from_millis(200)) else { continue };
                // The call's capture can restart at another rate, on another mic
                let down = match &mut down {
                    Some(d) if d.rate == rate => d,
                    _ => down.insert(Downsampler::new(rate)),
                };
                down.push(&chunk, &mut pcm);
                while pcm.len() >= CHUNK {
                    detector.push(pcm.drain(..CHUNK).collect(), &mut events);
                }
                for event in events.drain(..) {
                    match event {
                        Event::Start => {
                            sending = now_ms() >= paused_until.load(Ordering::Relaxed);
                            if sending {
                                recognizer.send(Out::Start(lang));
                            }
                        }
                        Event::Audio(block) if sending => recognizer.send(Out::Audio(block)),
                        Event::End(speech) if sending => recognizer.send(if speech { Out::End } else { Out::Cancel }),
                        _ => {}
                    }
                }
            }
            detector.stop(&mut events);
            if sending && !events.is_empty() {
                recognizer.send(Out::Cancel);
            }
            drop(source);
        })
        .map_err(|e| e.to_string())?;
    opened.recv().map_err(|_| "the speech thread ended".to_owned())??;
    Ok(Recording { stop })
}

/// The mic's rate down to 16 kHz, like the web's `pcm.ts`: each output sample averages the input samples it
/// covers, which also filters what's above the new rate.
struct Downsampler {
    rate: u32,
    ratio: f64,
    carry: Vec<f32>,
    /// Where the next output sample starts in `carry`, in input samples
    at: f64,
}

impl Downsampler {
    fn new(rate: u32) -> Self {
        Self { rate, ratio: (f64::from(rate) / f64::from(TRANSCRIBE_SAMPLE_RATE)).max(1.0), carry: Vec::new(), at: 0.0 }
    }

    fn push(&mut self, samples: &[f32], out: &mut Vec<i16>) {
        self.carry.extend_from_slice(samples);
        loop {
            let start = self.at as usize;
            let end = ((self.at + self.ratio) as usize).max(start + 1);
            let Some(span) = self.carry.get(start..end) else { break };
            let mean = span.iter().sum::<f32>() / span.len() as f32;
            out.push((mean.clamp(-1.0, 1.0) * 32767.0) as i16);
            self.at += self.ratio;
        }
        let used = (self.at as usize).min(self.carry.len());
        self.carry.drain(..used);
        self.at -= used as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::Downsampler;

    #[test]
    fn downsampling_48k_keeps_duration_across_odd_chunks() {
        let mut down = Downsampler::new(48_000);
        let mut out = Vec::new();
        // One second in uneven pieces, as audio callbacks deliver it
        for size in [441, 1000, 7, 46_552] {
            down.push(&vec![0.5; size], &mut out);
        }
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| s == (0.5 * 32767.0) as i16));
    }

    #[test]
    fn downsampling_44_1k_rounds_to_the_right_length() {
        let mut down = Downsampler::new(44_100);
        let mut out = Vec::new();
        down.push(&vec![0.0; 44_100], &mut out);
        assert!((15_999..=16_000).contains(&out.len()), "{}", out.len());
    }
}
