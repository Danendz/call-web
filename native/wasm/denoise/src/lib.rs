//! Plain C exports, no wasm-bindgen: the AudioWorklet instantiates the module with no imports and calls these.
//! One instance per module: the worklet makes one module per mic.

use denoise::{Denoiser, FRAME};

static mut DENOISER: Option<Denoiser> = None;
static mut BUFFER: [f32; FRAME] = [0.0; FRAME];

/// Loads the model; 0 if it failed.
#[unsafe(no_mangle)]
pub extern "C" fn denoise_init() -> i32 {
    // SAFETY: WASM in a worklet is single-threaded, and nothing holds a reference across these calls
    unsafe { (*&raw mut DENOISER) = Denoiser::new().ok() };
    unsafe { (*&raw const DENOISER).is_some() as i32 }
}

/// Where the worklet writes a frame of 480 samples (-1..1) and reads it back cleaned.
#[unsafe(no_mangle)]
pub extern "C" fn denoise_buffer() -> *mut f32 {
    (&raw mut BUFFER).cast()
}

/// Cleans the frame in the buffer.
#[unsafe(no_mangle)]
pub extern "C" fn denoise_process() {
    // SAFETY: as in `denoise_init`
    unsafe {
        if let Some(denoiser) = (*&raw mut DENOISER).as_mut() {
            denoiser.process(&mut *&raw mut BUFFER, 1.0);
        }
    }
}
