//! `tracing` into logcat (`adb logcat -s call-danya`), through the NDK's liblog: stdout goes nowhere on Android.

use std::ffi::{CString, c_char, c_int};
use std::io;

const TAG: &core::ffi::CStr = c"call-danya";
/// `ANDROID_LOG_INFO`; the level is in the line itself
const PRIORITY: c_int = 4;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

pub fn init() {
    // `nativeInit` runs again when the activity is recreated; the first subscriber stays
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .with_ansi(false)
        .without_time()
        .with_writer(Line::default)
        .try_init();
}

/// One formatted event, written to logcat when the formatter drops it.
#[derive(Default)]
struct Line(Vec<u8>);

impl io::Write for Line {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        self.0.retain(|&b| b != 0);
        while self.0.last() == Some(&b'\n') {
            self.0.pop();
        }
        if let Ok(text) = CString::new(std::mem::take(&mut self.0)) {
            // SAFETY: both pointers are NUL-terminated strings that outlive the call
            unsafe { __android_log_write(PRIORITY, TAG.as_ptr(), text.as_ptr()) };
        }
    }
}
