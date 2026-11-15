//! Firefox's translation engine, bergamot-translator, built natively (see build.rs), behind a small C shim
//! (`cpp/shim.cpp`). The engine side (`engine::translate`) gives it the same model files and settings as its WASM build.

use std::ffi::{CStr, CString, c_char};
use std::path::Path;
use std::ptr::{self, NonNull};

#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn bergamot_new(
        config: *const c_char,
        model: *const c_char,
        lex: *const c_char,
        src_vocab: *const c_char,
        trg_vocab: *const c_char,
        error: *mut *mut c_char,
    ) -> *mut Raw;
    fn bergamot_translate(
        translator: *mut Raw,
        text: *const c_char,
        len: usize,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn bergamot_free_string(s: *mut c_char);
    fn bergamot_delete(translator: *mut Raw);
}

/// One direction's model, loaded. Not shared between threads: the engine keeps per-call state.
pub struct Translator {
    raw: NonNull<Raw>,
}

// SAFETY: the engine has no thread affinity; it's only used from one thread at a time, which `&mut self` ensures
unsafe impl Send for Translator {}

/// What one direction's model is made of
pub struct Files<'a> {
    pub model: &'a Path,
    pub lex: &'a Path,
    pub src_vocab: &'a Path,
    /// None when one vocabulary serves both languages
    pub trg_vocab: Option<&'a Path>,
}

impl Translator {
    /// Loads a model. `config` is Marian's YAML settings.
    pub fn load(config: &str, files: &Files) -> Result<Self, String> {
        let c = |s: &str| CString::new(s).map_err(|_| format!("{s:?} has a NUL in it"));
        let path = |p: &Path| p.to_str().ok_or_else(|| format!("{} isn't UTF-8", p.display())).and_then(c);
        let config = c(config)?;
        let (model, lex, src_vocab) = (path(files.model)?, path(files.lex)?, path(files.src_vocab)?);
        let trg_vocab = files.trg_vocab.map(path).transpose()?;
        let mut error = ptr::null_mut();
        // SAFETY: every pointer is a live NUL-terminated string or null where the shim allows it; it copies what it
        // keeps
        let raw = unsafe {
            bergamot_new(
                config.as_ptr(),
                model.as_ptr(),
                lex.as_ptr(),
                src_vocab.as_ptr(),
                trg_vocab.as_ref().map_or(ptr::null(), |v| v.as_ptr()),
                &raw mut error,
            )
        };
        NonNull::new(raw).map(|raw| Self { raw }).ok_or_else(|| take_error(error))
    }

    /// Translates one line.
    pub fn translate(&mut self, text: &str) -> Result<String, String> {
        let mut error = ptr::null_mut();
        // SAFETY: `raw` is live until drop; the text is passed with its length, so it needs no NUL
        let out = unsafe { bergamot_translate(self.raw.as_ptr(), text.as_ptr().cast(), text.len(), &raw mut error) };
        if out.is_null() {
            return Err(take_error(error));
        }
        // SAFETY: the shim returns a NUL-terminated copy, ours to free
        let translated = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        // SAFETY: as above
        unsafe { bergamot_free_string(out) };
        Ok(translated)
    }
}

impl Drop for Translator {
    fn drop(&mut self) {
        // SAFETY: made by `bergamot_new` and not freed before
        unsafe { bergamot_delete(self.raw.as_ptr()) };
    }
}

fn take_error(error: *mut c_char) -> String {
    if error.is_null() {
        return "unknown error".into();
    }
    // SAFETY: the shim sets a NUL-terminated copy, ours to free
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    // SAFETY: as above
    unsafe { bergamot_free_string(error) };
    text
}
