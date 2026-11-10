//! The translation model's side of the controller: which one `lang` needs, and its download. Translating itself
//! happens in the app, like the web's subtitles, with `crate::Translator`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use protocol::Lang;

use super::{Controller, Internal};
use crate::download::{self, ModelStatus};
use crate::translate::{Pair, model_dir, model_ready};

/// Whether the model `lang` needs is there
pub(super) fn status(data_dir: Option<&Path>, lang: Lang) -> ModelStatus {
    match data_dir {
        Some(d) if model_ready(&model_dir(d, Pair::into(lang)), Pair::into(lang)) => ModelStatus::Ready,
        _ => ModelStatus::Missing,
    }
}

impl Controller {
    pub(super) fn set_lang(&mut self, lang: Lang) {
        if Pair::into(lang) != Pair::into(self.state.lang) {
            // A download for the other language stops; what it got is kept for next time
            if let Some(cancel) = self.translation_cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.state.translation = status(self.data_dir.as_deref(), lang);
        }
        self.state.lang = lang;
    }

    pub(super) fn download_translation(&mut self) {
        let Some(data_dir) = self.data_dir.clone() else { return };
        if self.translation_cancel.is_some() || self.state.translation == ModelStatus::Ready {
            return;
        }
        let pair = Pair::into(self.state.lang);
        let cancel = Arc::new(AtomicBool::new(false));
        self.translation_cancel = Some(cancel.clone());
        self.state.translation = ModelStatus::Downloading(0);
        let (server, tx) = (self.server.clone(), self.internal_tx.clone());
        let spawned = std::thread::Builder::new().name("translation download".into()).spawn(move || {
            let progress = |percent| {
                let _ = tx.send(Internal::TranslationProgress(cancel.clone(), percent));
            };
            let dir = model_dir(&data_dir, pair);
            let result = download::download(&server, &dir, pair.files(), &cancel, &progress);
            let _ = tx.send(Internal::TranslationDownloaded(cancel.clone(), result));
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the translation download: {e}");
            self.translation_cancel = None;
            self.state.translation = ModelStatus::Failed;
        }
    }

    /// Whether `download` is the one running: a cancelled one can still report after a new one started.
    pub(super) fn is_current_translation(&self, download: &Arc<AtomicBool>) -> bool {
        self.translation_cancel.as_ref().is_some_and(|c| Arc::ptr_eq(c, download))
    }

    pub(super) fn translation_downloaded(&mut self, download: &Arc<AtomicBool>, result: Result<bool, String>) {
        if !self.is_current_translation(download) {
            return;
        }
        self.translation_cancel = None;
        self.state.translation = match result {
            Ok(true) => ModelStatus::Ready,
            Ok(false) => ModelStatus::Missing,
            Err(e) => {
                tracing::warn!("translation download failed: {e}");
                ModelStatus::Failed
            }
        };
    }

    pub(super) fn delete_translation(&mut self) {
        let Some(data_dir) = &self.data_dir else { return };
        if let Some(cancel) = self.translation_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        // The app drops its translator once this isn't Ready; it read the files whole when it loaded, so none is held open
        if let Err(e) = download::delete(&model_dir(data_dir, Pair::into(self.state.lang))) {
            tracing::warn!("couldn't delete the translation model: {e}");
        }
        self.state.translation = status(Some(data_dir), self.state.lang);
    }
}
