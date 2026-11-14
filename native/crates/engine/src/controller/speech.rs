//! Speech to chat's side of the controller: which recognizer a recording uses, what comes back from it, and the
//! on-device models' downloads.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use protocol::{ClientMessage, MAX_CHAT_LENGTH, MAX_LIVE_LENGTH, SpeechServerMessage};

use super::{Controller, Internal, Phase};
use crate::speech::{self, Model, ModelStatus, Params, SpeechEvent, SpeechNotice};

impl Controller {
    /// Starts or stops the recording; only while someone is there to read it, and never while muted, like the web.
    pub(super) fn sync_speech(&mut self) {
        let speaking = self.state.transcribe_on && self.state.mic_on && self.state.phase == Phase::InCall;
        let params = speaking.then(|| self.speech_params()).flatten();
        self.transcriber.sync(params);
        // The server's notices are about the server
        if self.transcriber.is_local() {
            self.state.speech_notice = None;
        }
    }

    fn speech_params(&self) -> Option<Params> {
        let (room, token) = (self.state.room_id.as_ref()?, self.seat.token.as_ref()?);
        let mut url = self.server.clone();
        // Only http and https servers are accepted at startup, so this can't fail
        let _ = url.set_scheme(if self.server.scheme() == "https" { "wss" } else { "ws" });
        url.set_path("/ws/speech");
        url.query_pairs_mut().clear().append_pair("room", room).append_pair("token", token);
        let mic = self.state.mic_id.as_ref().and_then(|id| self.state.mics.iter().find(|d| &d.id == id));
        let on_device = self.state.on_device && self.state.model == ModelStatus::Ready;
        let local = on_device.then(|| self.model_dir(Model::SenseVoice)).flatten();
        let live = self.state.live_subtitles;
        Some(Params { url, lang: self.state.lang, mic: mic.map(|d| d.label.clone()), local, live })
    }

    pub(super) fn speech_event(&mut self, event: SpeechEvent) {
        match event {
            SpeechEvent::Server(msg) => self.speech_notice(msg),
            SpeechEvent::Text { text, live_id } => {
                if let Some(signaling) = &self.signaling {
                    let text = cut(&text, MAX_CHAT_LENGTH);
                    signaling.send(ClientMessage::Chat { text, spoken: Some(true), live_id });
                }
            }
            SpeechEvent::Live { id, text } => {
                if let Some(signaling) = &self.signaling {
                    let text = cut(&text, MAX_LIVE_LENGTH);
                    signaling.send(ClientMessage::ChatLive { id, text, lang: self.state.lang });
                }
            }
            SpeechEvent::LocalFailed => self.transcriber.local_failed(),
        }
    }

    fn speech_notice(&mut self, msg: SpeechServerMessage) {
        let Some(notice) = self.transcriber.notice(msg) else { return };
        if self.transcriber.is_local() {
            return;
        }
        if let SpeechNotice::Paused { until_ms } = notice {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let wait = Duration::from_millis(until_ms.saturating_sub(now.as_millis() as u64));
            let tx = self.internal_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let _ = tx.send(Internal::SpeechResumed);
            });
        }
        self.state.speech_notice = Some(notice);
    }

    /// The model's status and the cancel flag of its running download
    fn model_slot(&mut self, model: Model) -> (&mut ModelStatus, &mut Option<Arc<AtomicBool>>) {
        match model {
            Model::SenseVoice => (&mut self.state.model, &mut self.model_cancel),
        }
    }

    pub(super) fn model_dir(&self, model: Model) -> Option<PathBuf> {
        self.data_dir.as_deref().map(|d| speech::model_dir(d, model))
    }

    pub(super) fn download_model(&mut self, model: Model) {
        let Some(dir) = self.model_dir(model) else { return };
        let (status, running) = self.model_slot(model);
        if running.is_some() || *status == ModelStatus::Ready {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *running = Some(cancel.clone());
        *status = ModelStatus::Downloading(0);
        let (server, tx) = (self.server.clone(), self.internal_tx.clone());
        let spawned = std::thread::Builder::new().name("model download".into()).spawn(move || {
            let progress = |percent| {
                let _ = tx.send(Internal::ModelProgress(model, cancel.clone(), percent));
            };
            let result = speech::download_model(&server, &dir, model, &cancel, &progress);
            let _ = tx.send(Internal::ModelDownloaded(model, cancel.clone(), result));
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the model download: {e}");
            let (status, running) = self.model_slot(model);
            *running = None;
            *status = ModelStatus::Failed;
        }
    }

    pub(super) fn model_progress(&mut self, model: Model, download: &Arc<AtomicBool>, percent: u8) {
        let (status, running) = self.model_slot(model);
        // A cancelled download can still report after a new one started
        if running.as_ref().is_some_and(|c| Arc::ptr_eq(c, download)) {
            *status = ModelStatus::Downloading(percent);
        }
    }

    pub(super) fn model_downloaded(&mut self, model: Model, download: &Arc<AtomicBool>, result: Result<bool, String>) {
        let (status, running) = self.model_slot(model);
        if !running.as_ref().is_some_and(|c| Arc::ptr_eq(c, download)) {
            return;
        }
        *running = None;
        *status = match result {
            Ok(true) => ModelStatus::Ready,
            Ok(false) => ModelStatus::Missing,
            Err(e) => {
                tracing::warn!("{model:?} model download failed: {e}");
                ModelStatus::Failed
            }
        };
    }

    pub(super) fn cancel_model_download(&mut self, model: Model) {
        let (status, running) = self.model_slot(model);
        if let Some(cancel) = running.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if *status != ModelStatus::Ready {
            *status = ModelStatus::Missing;
        }
    }

    pub(super) fn delete_model(&mut self, model: Model) {
        let Some(dir) = self.model_dir(model) else { return };
        self.cancel_model_download(model);
        self.transcriber.release();
        if let Err(e) = speech::delete_model(&dir) {
            tracing::warn!("couldn't delete the {model:?} model: {e}");
        }
        let ready = speech::model_ready(&dir, model);
        *self.model_slot(model).0 = if ready { ModelStatus::Ready } else { ModelStatus::Missing };
    }
}

/// Removes the old live model; true when there was one.
pub(super) fn drop_old_live(data_dir: Option<&Path>) -> bool {
    data_dir.is_some_and(speech::drop_old_live)
}

/// SenseVoice is there but for the VAD that came with it later, which makes it whole
pub(super) fn lacks_vad(data_dir: Option<&Path>) -> bool {
    data_dir.is_some_and(|d| speech::lacks_vad(&speech::model_dir(d, Model::SenseVoice)))
}

/// Whether `model` is all there in the data folder
pub(super) fn status(data_dir: Option<&Path>, model: Model) -> ModelStatus {
    match data_dir {
        Some(d) if speech::model_ready(&speech::model_dir(d, model), model) => ModelStatus::Ready,
        _ => ModelStatus::Missing,
    }
}

/// `text` cut to what the server takes, which counts UTF-16 units
fn cut(text: &str, max: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}
