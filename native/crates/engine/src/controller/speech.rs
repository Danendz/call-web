//! Speech to chat's side of the controller: which recognizer a recording uses, what comes back from it, and the
//! on-device model's download.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use protocol::{ClientMessage, MAX_CHAT_LENGTH, SpeechServerMessage};

use super::{Controller, Internal, Phase};
use crate::speech::{self, ModelStatus, Params, SpeechEvent, SpeechNotice};

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
        let local =
            (self.state.on_device && self.state.model == ModelStatus::Ready).then(|| self.model_dir.clone()).flatten();
        Some(Params { url, lang: self.state.lang, mic: mic.map(|d| d.label.clone()), local })
    }

    pub(super) fn speech_event(&mut self, event: SpeechEvent) {
        match event {
            SpeechEvent::Server(msg) => self.speech_notice(msg),
            SpeechEvent::Text(text) => {
                // Cut to what the server takes, which counts UTF-16 units
                let mut units = 0;
                let text: String = text
                    .chars()
                    .take_while(|c| {
                        units += c.len_utf16();
                        units <= MAX_CHAT_LENGTH
                    })
                    .collect();
                if let Some(signaling) = &self.signaling {
                    signaling.send(ClientMessage::Chat { text, spoken: Some(true) });
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

    pub(super) fn download_model(&mut self) {
        let Some(dir) = self.model_dir.clone() else { return };
        if self.model_cancel.is_some() || self.state.model == ModelStatus::Ready {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.model_cancel = Some(cancel.clone());
        self.state.model = ModelStatus::Downloading(0);
        let (server, tx) = (self.server.clone(), self.internal_tx.clone());
        let spawned = std::thread::Builder::new().name("model download".into()).spawn(move || {
            let progress = |percent| {
                let _ = tx.send(Internal::ModelProgress(cancel.clone(), percent));
            };
            let result = speech::download_model(&server, &dir, &cancel, &progress);
            let _ = tx.send(Internal::ModelDownloaded(cancel.clone(), result));
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the model download: {e}");
            self.model_cancel = None;
            self.state.model = ModelStatus::Failed;
        }
    }

    /// Whether `download` is the one running: a cancelled one can still report after a new one started.
    pub(super) fn is_current_download(&self, download: &Arc<AtomicBool>) -> bool {
        self.model_cancel.as_ref().is_some_and(|c| Arc::ptr_eq(c, download))
    }

    pub(super) fn model_downloaded(&mut self, download: &Arc<AtomicBool>, result: Result<bool, String>) {
        if !self.is_current_download(download) {
            return;
        }
        self.model_cancel = None;
        self.state.model = match result {
            Ok(true) => ModelStatus::Ready,
            Ok(false) => ModelStatus::Missing,
            Err(e) => {
                tracing::warn!("model download failed: {e}");
                ModelStatus::Failed
            }
        };
    }

    pub(super) fn delete_model(&mut self) {
        let Some(dir) = &self.model_dir else { return };
        if let Some(cancel) = self.model_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.transcriber.release();
        if let Err(e) = speech::delete_model(dir) {
            tracing::warn!("couldn't delete the model: {e}");
        }
        self.state.model = if speech::model_ready(dir) { ModelStatus::Ready } else { ModelStatus::Missing };
    }
}
