//! The on-device speech model's files: where they're kept, and whether they're all there. They're downloaded by
//! `crate::download`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use url::Url;

use crate::download::{self, ModelFile};

/// SenseVoice-Small, int8: Mandarin and English (and mixed), with punctuation.
const NAME: &str = "sensevoice-small-2024-07-17";

pub(crate) const MODEL: ModelFile = ModelFile {
    local: "model.int8.onnx",
    remote: "sensevoice-small-int8-2024-07-17.onnx",
    size: 239_233_841,
    sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
};

pub(crate) const TOKENS: ModelFile = ModelFile {
    local: "tokens.txt",
    remote: "sensevoice-small-2024-07-17-tokens.txt",
    size: 315_894,
    sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
};

const FILES: [&ModelFile; 2] = [&MODEL, &TOKENS];

/// The whole download, for the UI.
pub const MODEL_SIZE: u64 = MODEL.size + TOKENS.size;

/// The model's folder in the app's data folder.
pub(crate) fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(NAME)
}

pub(crate) fn is_ready(dir: &Path) -> bool {
    download::is_ready(dir, &FILES)
}

pub(crate) fn download(
    server: &Url,
    dir: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(u8),
) -> Result<bool, String> {
    download::download(server, dir, &FILES, cancel, progress)
}

pub(crate) use download::delete;
