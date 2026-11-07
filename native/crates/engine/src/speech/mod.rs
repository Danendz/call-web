//! Speech to chat: what you say becomes chat messages, through the server's `/ws/speech`. The mic is opened apart
//! from WebRTC's capture and run through WebRTC's audio processing with echo cancellation, fed the other person's
//! decoded audio as the echo to remove, so their voice from your speakers isn't transcribed as yours. Then it's cut
//! into utterances and streamed while you speak, like the web's server path.

mod socket;
mod utterance;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::native::apm::AudioProcessingModule;
use protocol::{Lang, SpeechServerMessage, TRANSCRIBE_SAMPLE_RATE};
use tokio::sync::mpsc;
use url::Url;

use socket::{Out, SpeechSocket};
use utterance::{CHUNK, Detector, Event};

const RATE: i32 = TRANSCRIBE_SAMPLE_RATE as i32;
const TEN_MS: usize = TRANSCRIBE_SAMPLE_RATE as usize / 100;

/// Something the person should know about speech to chat; the UI words it in their language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechNotice {
    /// The server has no speech model
    Unavailable,
    /// Out of quota until this epoch ms
    Paused { until_ms: u64 },
}

/// What a recording runs for; a change restarts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Params {
    /// `/ws/speech?room=&token=`
    pub url: Url,
    pub lang: Lang,
    /// The mic's name, as for the level meter
    pub mic: Option<String>,
}

pub(crate) struct Transcriber {
    apm: Arc<Mutex<AudioProcessingModule>>,
    running: Option<(Params, Recording)>,
    reverse: Option<tokio::task::JoinHandle<()>>,
    notices: mpsc::UnboundedSender<SpeechServerMessage>,
    paused_until: Arc<AtomicU64>,
    unavailable: bool,
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
    /// The server's notices come back through `notices`, for [`Transcriber::notice`] on the engine's thread.
    pub(crate) fn new(notices: mpsc::UnboundedSender<SpeechServerMessage>) -> Self {
        // Echo cancellation and noise suppression; gain control would pump the noise floor the detector relies on
        let apm = AudioProcessingModule::new(true, false, true, true);
        Self {
            apm: Arc::new(Mutex::new(apm)),
            running: None,
            reverse: None,
            notices,
            paused_until: Arc::default(),
            unavailable: false,
        }
    }

    /// Records for `params`, or stops when None. Cheap to call on every change: it only restarts when they differ.
    pub(crate) fn sync(&mut self, params: Option<Params>) {
        let params = params.filter(|_| !self.unavailable);
        if self.running.as_ref().map(|(p, _)| p) == params.as_ref() {
            return;
        }
        self.running = None;
        if let Some(params) = params {
            let socket = SpeechSocket::connect(params.url.clone(), self.notices.clone());
            match start(&params, socket, self.apm.clone(), self.paused_until.clone()) {
                Ok(recording) => self.running = Some((params, recording)),
                Err(e) => tracing::info!("speech to chat can't open the mic: {e}"),
            }
        }
    }

    /// The other person's voice, as the echo to take out of ours.
    pub(crate) fn set_remote_audio(&mut self, track: Option<RtcAudioTrack>) {
        if let Some(old) = self.reverse.take() {
            old.abort();
        }
        let Some(track) = track else { return };
        let apm = self.apm.clone();
        self.reverse = Some(tokio::spawn(async move {
            let mut stream = NativeAudioStream::new(track, RATE, 1);
            let mut pending: Vec<i16> = Vec::new();
            while let Some(frame) = stream.next().await {
                pending.extend_from_slice(&frame.data);
                let whole = pending.len() / TEN_MS * TEN_MS;
                if whole > 0 {
                    if let Ok(mut apm) = apm.lock()
                        && let Some(slice) = pending.get_mut(..whole)
                    {
                        let _ = apm.process_reverse_stream(slice, RATE, 1);
                    }
                    pending.drain(..whole);
                }
            }
        }));
    }

    /// A notice from the server; returns what the UI should show, if it changed.
    pub(crate) fn notice(&mut self, msg: SpeechServerMessage) -> Option<SpeechNotice> {
        match msg {
            SpeechServerMessage::Unavailable => {
                self.unavailable = true;
                self.running = None;
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

/// Opens the mic on a thread of its own and runs samples through the pipeline until the Recording is dropped.
fn start(
    params: &Params,
    socket: SpeechSocket,
    apm: Arc<Mutex<AudioProcessingModule>>,
    paused_until: Arc<AtomicU64>,
) -> Result<Recording, String> {
    let (stop, stopped) = std_mpsc::channel::<()>();
    let (opened_tx, opened) = std_mpsc::channel();
    let (mic, lang) = (params.mic.clone(), params.lang);
    thread::Builder::new()
        .name("speech".into())
        .spawn(move || {
            let (samples_tx, samples) = std_mpsc::channel::<Vec<f32>>();
            let input = match crate::mic::open(mic.as_deref(), move |s| {
                let _ = samples_tx.send(s.to_vec());
            }) {
                Ok(input) => input,
                Err(e) => {
                    let _ = opened_tx.send(Err(e));
                    return;
                }
            };
            let _ = opened_tx.send(Ok(()));
            let mut down = Downsampler::new(input.sample_rate);
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
                let Ok(chunk) = samples.recv_timeout(std::time::Duration::from_millis(200)) else { continue };
                down.push(&chunk, &mut pcm);
                while pcm.len() >= CHUNK {
                    let mut block: Vec<i16> = pcm.drain(..CHUNK).collect();
                    if let Ok(mut apm) = apm.lock() {
                        let _ = apm.process_stream(&mut block, RATE, 1);
                    }
                    detector.push(block, &mut events);
                }
                for event in events.drain(..) {
                    match event {
                        Event::Start => {
                            sending = now_ms() >= paused_until.load(Ordering::Relaxed);
                            if sending {
                                socket.send(Out::Start(lang));
                            }
                        }
                        Event::Audio(block) if sending => socket.send(Out::Audio(block)),
                        Event::End(speech) if sending => socket.send(if speech { Out::End } else { Out::Cancel }),
                        _ => {}
                    }
                }
            }
            detector.stop(&mut events);
            if sending && !events.is_empty() {
                socket.send(Out::Cancel);
            }
            drop(input);
        })
        .map_err(|e| e.to_string())?;
    opened.recv().map_err(|_| "the speech thread ended".to_owned())??;
    Ok(Recording { stop })
}

/// The mic's rate down to 16 kHz, like the web's `pcm.ts`: each output sample averages the input samples it
/// covers, which also filters what's above the new rate.
struct Downsampler {
    ratio: f64,
    carry: Vec<f32>,
    /// Where the next output sample starts in `carry`, in input samples
    at: f64,
}

impl Downsampler {
    fn new(rate: u32) -> Self {
        Self { ratio: (f64::from(rate) / f64::from(TRANSCRIBE_SAMPLE_RATE)).max(1.0), carry: Vec::new(), at: 0.0 }
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
