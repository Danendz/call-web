//! Firefox's translation engine, bergamot-translator, built natively (see build.rs), behind a small C shim
//! (`cpp/shim.cpp`). The engine side (`engine::translate`) gives it the same model files and settings as its WASM build.

use std::ffi::{CStr, CString, c_char};
use std::ptr::{self, NonNull};

#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

/// A file's bytes, borrowed for the call
#[repr(C)]
struct Bytes {
    data: *const c_char,
    len: usize,
}

impl Bytes {
    fn of(bytes: &[u8]) -> Self {
        Self { data: bytes.as_ptr().cast(), len: bytes.len() }
    }
}

unsafe extern "C" {
    fn bergamot_new(
        config: *const c_char,
        model: Bytes,
        lex: Bytes,
        src_vocab: Bytes,
        trg_vocab: Bytes,
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

/// What one direction's model is made of: its files' contents, which the engine copies
pub struct Files<'a> {
    pub model: &'a [u8],
    pub lex: &'a [u8],
    pub src_vocab: &'a [u8],
    /// None when one vocabulary serves both languages
    pub trg_vocab: Option<&'a [u8]>,
}

impl Translator {
    /// Loads a model. `config` is Marian's YAML settings.
    pub fn load(config: &str, files: &Files) -> Result<Self, String> {
        let config = CString::new(config).map_err(|_| "the config has a NUL in it".to_string())?;
        let trg_vocab = files.trg_vocab.map_or(Bytes { data: ptr::null(), len: 0 }, Bytes::of);
        let mut error = ptr::null_mut();
        // SAFETY: the config is a live NUL-terminated string and each file's bytes are live for the call, with their
        // length (a null target vocabulary is allowed); the shim copies what it keeps
        let raw = unsafe {
            bergamot_new(
                config.as_ptr(),
                Bytes::of(files.model),
                Bytes::of(files.lex),
                Bytes::of(files.src_vocab),
                trg_vocab,
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
