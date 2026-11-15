//! Bergamot built natively (crates/bergamot), the app's engine on every platform; `host.rs` runs its WebAssembly build
//! instead with the `wasm-translate` feature. Same model files and settings as there.

use std::path::Path;

use super::CONFIG;
use super::model::{self, Pair};

pub(crate) struct Bergamot(bergamot::Translator);

impl Bergamot {
    pub(crate) fn load(dir: &Path, _pair: Pair) -> Result<Self, String> {
        let read = |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"));
        let (model, lex) = (read(model::MODEL)?, read(model::LEX)?);
        let (src_vocab, trg_vocab) = if dir.join(model::VOCAB).exists() {
            (read(model::VOCAB)?, None)
        } else {
            (read(model::SRC_VOCAB)?, Some(read(model::TRG_VOCAB)?))
        };
        let mut config: String = CONFIG.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
        // Marian reads a vocabulary's name by index even when they come from memory, where the list is empty: WASM
        // reads zeros past its end, a native build crashes. Only factored models use the names
        config += "vocabs: [src.spm, trg.spm]\n";
        let files =
            bergamot::Files { model: &model, lex: &lex, src_vocab: &src_vocab, trg_vocab: trg_vocab.as_deref() };
        bergamot::Translator::load(&config, &files).map(Self)
    }

    /// Translates one line.
    pub(crate) fn translate(&mut self, text: &str) -> Result<String, String> {
        self.0.translate(text)
    }
}
