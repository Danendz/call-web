//! The on-device speech models' files: where they're kept, and whether they're all there. They're downloaded by
//! `crate::download`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use url::Url;

use crate::download::{self, ModelFile};

/// A speech model on this device, each in a folder of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// SenseVoice-Small, int8: Mandarin and English (and mixed), with punctuation. Decodes each utterance once it's
    /// over, or for live subtitles the line so far as it grows, with Silero VAD cutting the lines.
    SenseVoice,
}

pub(crate) const SENSE_VOICE: ModelFile = ModelFile {
    local: "model.int8.onnx",
    remote: "sensevoice-small-int8-2024-07-17.onnx",
    size: 239_233_841,
    sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
};

pub(crate) const SENSE_VOICE_TOKENS: ModelFile = ModelFile {
    local: "tokens.txt",
    remote: "sensevoice-small-2024-07-17-tokens.txt",
    size: 315_894,
    sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
};

// Where speech is, for cutting live subtitles into lines (`live.rs`)
pub(crate) const SILERO_VAD: ModelFile = ModelFile {
    local: "silero-vad.onnx",
    remote: "silero-vad-v4.onnx",
    size: 643_854,
    sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
};

// The streaming model live subtitles used up to 0.3.2, in its own folder; SenseVoice does it now
const OLD_LIVE: &str = "live-zh-en-2023-02-20";

impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::SenseVoice => "sensevoice-small-2024-07-17",
        }
    }

    fn files(self) -> &'static [&'static ModelFile] {
        match self {
            Self::SenseVoice => &[&SENSE_VOICE, &SENSE_VOICE_TOKENS, &SILERO_VAD],
        }
    }

    /// The whole download, for the UI
    pub fn size(self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

/// The model's folder in the app's data folder.
pub(crate) fn dir(data_dir: &Path, model: Model) -> PathBuf {
    data_dir.join("models").join(model.name())
}

pub(crate) fn is_ready(dir: &Path, model: Model) -> bool {
    download::is_ready(dir, model.files())
}

pub(crate) fn download(
    server: &Url,
    dir: &Path,
    model: Model,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(u8),
) -> Result<bool, String> {
    download::download(server, dir, model.files(), cancel, progress)
}

pub(crate) use download::delete;

/// Removes the old live model (~330 MB) from the app's data folder; true when there was one.
pub(crate) fn drop_old_live(data_dir: &Path) -> bool {
    std::fs::remove_dir_all(data_dir.join("models").join(OLD_LIVE)).is_ok()
}

/// SenseVoice downloaded before it came with Silero: all there but the VAD, a small download to make it whole.
pub(crate) fn lacks_vad(dir: &Path) -> bool {
    dir.join(SENSE_VOICE.local).exists() && !dir.join(SILERO_VAD.local).exists()
}
