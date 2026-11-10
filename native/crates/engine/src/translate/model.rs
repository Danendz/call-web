//! The translation model for each direction: Firefox's Bergamot models, ~52 MB each, downloaded by `crate::download`
//! into the app's data folder. Kept the same as web/src/translate/models.ts.

use std::path::{Path, PathBuf};

use protocol::Lang;

use crate::download::{self, ModelFile};

/// A direction, from one language into the other. Each device only needs the one into the language it speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    EnZh,
    ZhEn,
}

impl Pair {
    /// From the other language into `lang`
    pub fn into(lang: Lang) -> Self {
        match lang {
            Lang::Zh => Pair::EnZh,
            Lang::En => Pair::ZhEn,
        }
    }

    /// The languages as Bergamot names them
    pub(crate) fn langs(self) -> (&'static str, &'static str) {
        match self {
            Pair::EnZh => ("en", "zh"),
            Pair::ZhEn => ("zh", "en"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Pair::EnZh => "bergamot-enzh-2.2",
            Pair::ZhEn => "bergamot-zhen-2.1",
        }
    }

    pub(crate) fn files(self) -> &'static [&'static ModelFile] {
        match self {
            Pair::EnZh => &EN_ZH,
            Pair::ZhEn => &ZH_EN,
        }
    }

    /// The whole download, for the UI
    pub fn size(self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

/// What each file is to the engine, by its name in the model's folder
pub(crate) const MODEL: &str = "model.bin";
pub(crate) const LEX: &str = "lex.bin";
pub(crate) const VOCAB: &str = "vocab.spm";
pub(crate) const SRC_VOCAB: &str = "srcvocab.spm";
pub(crate) const TRG_VOCAB: &str = "trgvocab.spm";

const EN_ZH: [&ModelFile; 4] = [
    &ModelFile {
        local: MODEL,
        remote: "bergamot-enzh-2.2-model.bin",
        size: 43_849_787,
        sha256: "4e5accc141373565ddc8fa1565bceaa8d0c3482a82cab8131c719ebcc6c2157c",
    },
    &ModelFile {
        local: LEX,
        remote: "bergamot-enzh-2.2-lex.bin",
        size: 6_506_248,
        sha256: "4a5e5827788060f1d718a8132b69440929387514a045796e9b77f935db68c055",
    },
    &ModelFile {
        local: SRC_VOCAB,
        remote: "bergamot-enzh-2.2-srcvocab.spm",
        size: 806_952,
        sha256: "bd9b65504acc6d9726dd281f7defc2adb7c2c22d0688fe2f84697de25197c8c5",
    },
    &ModelFile {
        local: TRG_VOCAB,
        remote: "bergamot-enzh-2.2-trgvocab.spm",
        size: 772_004,
        sha256: "aded6993c36e440284d11cec3f6b8aef9c0e43188a772d80be342a713adf223d",
    },
];

const ZH_EN: [&ModelFile; 3] = [
    &ModelFile {
        local: MODEL,
        remote: "bergamot-zhen-2.1-model.bin",
        size: 43_977_787,
        sha256: "5cd149601802fc8a18124a1c1306144dbbedc058630c4ddb2d53aa76fa9c7c06",
    },
    &ModelFile {
        local: LEX,
        remote: "bergamot-zhen-2.1-lex.bin",
        size: 9_219_192,
        sha256: "8524dd4c93ffd7f0ae7be32f77a7c14fea7cdbbebe34ea6ad0ec368d5ddd8b13",
    },
    &ModelFile {
        local: VOCAB,
        remote: "bergamot-zhen-2.1-vocab.spm",
        size: 1_359_697,
        sha256: "dff594318ab7d8b7b60b844ab98ebe6b932ae8045fab15235404c787715965b3",
    },
];

/// The direction's folder in the app's data folder
pub(crate) fn dir(data_dir: &Path, pair: Pair) -> PathBuf {
    data_dir.join("models").join(pair.name())
}

pub(crate) fn is_ready(dir: &Path, pair: Pair) -> bool {
    download::is_ready(dir, pair.files())
}
