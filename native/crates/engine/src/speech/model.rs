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
    /// SenseVoice-Small, int8: decodes each utterance once it's over. Mandarin and English (and mixed), with
    /// punctuation.
    SenseVoice,
    /// For live subtitles: a streaming Zipformer (Mandarin and English, mixed), int8 but for its decoder, that decodes
    /// while you speak, and a CT-Transformer that punctuates the line once it's over.
    Live,
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

pub(crate) const ENCODER: ModelFile = ModelFile {
    local: "encoder.int8.onnx",
    remote: "streaming-zipformer-zh-en-2023-02-20-encoder.int8.onnx",
    size: 181_895_032,
    sha256: "8fa764187a261844f859d7143ebaa563af5d10adfece4c18a8f414c88cba2a9b",
};

// Full precision: the int8 one repeats characters over and over ("凑凑凑凑够够") under greedy search
pub(crate) const DECODER: ModelFile = ModelFile {
    local: "decoder.onnx",
    remote: "streaming-zipformer-zh-en-2023-02-20-decoder.onnx",
    size: 13_876_452,
    sha256: "2e3b5ec371f8899ee6acd829fd753ba45772df57a91bdf37cde3136354e7db7d",
};

// The int8 decoder that live models downloaded up to 0.3.2 have in place of `DECODER`
const OLD_DECODER: &str = "decoder.int8.onnx";

pub(crate) const JOINER: ModelFile = ModelFile {
    local: "joiner.int8.onnx",
    remote: "streaming-zipformer-zh-en-2023-02-20-joiner.int8.onnx",
    size: 3_228_404,
    sha256: "1ed689c5ed19dbaa725d9d191bb4822b5f4855a39e1ffd28cbc1f340d25b2ee0",
};

pub(crate) const LIVE_TOKENS: ModelFile = ModelFile {
    local: "tokens.txt",
    remote: "streaming-zipformer-zh-en-2023-02-20-tokens.txt",
    size: 56_317,
    sha256: "a8e0e4ec53810e433789b54a5c0134a7eaa2ffca595a6334d54c00da858841d3",
};

pub(crate) const PUNCTUATION: ModelFile = ModelFile {
    local: "punctuation.int8.onnx",
    remote: "ct-transformer-zh-en-2024-04-12-punctuation.int8.onnx",
    size: 75_519_198,
    sha256: "65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1",
};

impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::SenseVoice => "sensevoice-small-2024-07-17",
            Self::Live => "live-zh-en-2023-02-20",
        }
    }

    fn files(self) -> &'static [&'static ModelFile] {
        match self {
            Self::SenseVoice => &[&SENSE_VOICE, &SENSE_VOICE_TOKENS],
            Self::Live => &[&ENCODER, &DECODER, &JOINER, &LIVE_TOKENS, &PUNCTUATION],
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

/// Removes the int8 decoder of a live model downloaded before it changed; true when there was one, so the new
/// decoder should be fetched to make the model whole again.
pub(crate) fn drop_old_decoder(dir: &Path) -> bool {
    std::fs::remove_file(dir.join(OLD_DECODER)).is_ok()
}
