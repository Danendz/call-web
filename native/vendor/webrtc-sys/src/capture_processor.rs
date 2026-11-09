// Copyright 2025 LiveKit, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A hook into the microphone's audio after WebRTC's own processing (echo
//! cancellation, gain control), for custom processing such as ML noise
//! suppression, and a switch for WebRTC's built-in noise suppression.

use std::sync::Mutex;

/// Called on the audio thread with each 10 ms frame of the first channel, as
/// floats in the 16-bit range, and the sample rate. It must not block.
pub type CaptureProcessor = Box<dyn FnMut(&mut [f32], u32) + Send>;

static PROCESSOR: Mutex<Option<CaptureProcessor>> = Mutex::new(None);

#[cxx::bridge(namespace = "livekit_ffi")]
mod ffi {
    unsafe extern "C++" {
        include!("livekit/capture_processor.h");

        fn set_builtin_noise_suppression(enabled: bool);
    }

    extern "Rust" {
        fn process_capture(samples: &mut [f32], sample_rate: i32);
    }
}

/// Sets (or with None, removes) the processor for the microphone's audio.
/// Applies to every peer connection factory.
pub fn set_capture_processor(processor: Option<CaptureProcessor>) {
    *PROCESSOR.lock().unwrap_or_else(|e| e.into_inner()) = processor;
}

/// Turns WebRTC's own noise suppression on (the default) or off for the
/// microphone, live and for tracks made later.
pub fn set_builtin_noise_suppression(enabled: bool) {
    ffi::set_builtin_noise_suppression(enabled);
}

fn process_capture(samples: &mut [f32], sample_rate: i32) {
    // A frame passes through untouched rather than wait for a processor that
    // is being swapped
    let Ok(mut processor) = PROCESSOR.try_lock() else { return };
    if let (Some(processor), Ok(rate)) = (processor.as_mut(), u32::try_from(sample_rate)) {
        processor(samples, rate);
    }
}
