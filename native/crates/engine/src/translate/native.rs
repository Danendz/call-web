//! Bergamot built natively (crates/bergamot), where wasmtime can't run machine code: on iOS, and elsewhere with the
//! `native-translate` feature to compare the two. Same model files and settings as `host.rs`.

use std::path::Path;

use super::CONFIG;
use super::model::{self, Pair};

pub(crate) struct Bergamot(bergamot::Translator);

impl Bergamot {
    pub(crate) fn load(dir: &Path, _pair: Pair) -> Result<Self, String> {
        let (model, lex) = (dir.join(model::MODEL), dir.join(model::LEX));
        let (one, src, trg) = (dir.join(model::VOCAB), dir.join(model::SRC_VOCAB), dir.join(model::TRG_VOCAB));
        let (src_vocab, trg_vocab) = if one.exists() { (&one, None) } else { (&src, Some(trg.as_path())) };
        let mut config: String = CONFIG.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
        // Marian reads a vocabulary's name by index even when they come from memory, where the list is empty: WASM
        // reads zeros past its end, a native build crashes. Only factored models use the names
        let name = |p: &Path| format!("{:?}", p.display().to_string());
        config += &format!("vocabs: [{}, {}]\n", name(src_vocab), name(trg_vocab.unwrap_or(src_vocab)));
        let files = bergamot::Files { model: &model, lex: &lex, src_vocab, trg_vocab };
        bergamot::Translator::load(&config, &files).map(Self)
    }

    /// Translates one line.
    pub(crate) fn translate(&mut self, text: &str) -> Result<String, String> {
        self.0.translate(text)
    }
}
