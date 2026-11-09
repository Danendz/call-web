//! ML noise suppression for the mic: DeepFilterNet3, on 10 ms frames of 48 kHz mono. The native engine runs it inside
//! WebRTC's audio processing; the web app runs the same code as WASM in an AudioWorklet (`wasm/denoise`).

use deep_filter_rt::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in one frame: 10 ms
pub const FRAME: usize = 480;

pub struct Denoiser {
    df: DfTract,
    noisy: Array2<f32>,
    clean: Array2<f32>,
}

impl Denoiser {
    /// Loads the embedded model; takes a moment, so not on an audio thread.
    pub fn new() -> Result<Self, String> {
        let df = DfTract::new(DfParams::default(), &RuntimeParams::default()).map_err(|e| e.to_string())?;
        if df.hop_size != FRAME || df.sr != SAMPLE_RATE as usize {
            return Err(format!("the model takes {} samples at {} Hz", df.hop_size, df.sr));
        }
        Ok(Self { df, noisy: Array2::zeros((1, FRAME)), clean: Array2::zeros((1, FRAME)) })
    }

    /// Cleans one frame in place. `scale` is full scale for its samples (1 for -1..1, 32768 for WebRTC's floats in
    /// the 16-bit range). A frame of another size, or one the model fails on, passes unchanged.
    pub fn process(&mut self, frame: &mut [f32], scale: f32) {
        if frame.len() != FRAME {
            return;
        }
        for (dst, src) in self.noisy.iter_mut().zip(frame.iter()) {
            *dst = src / scale;
        }
        if self.run().is_err() {
            return;
        }
        for (dst, src) in frame.iter_mut().zip(self.clean.iter()) {
            *dst = src * scale;
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
    fn run(&mut self) -> Result<f32, impl std::fmt::Debug> {
        // SAFETY: the guard puts the float mode back before returning, and the model's math doesn't depend on
        // denormals
        unsafe { no_denormals::no_denormals(|| self.df.process(self.noisy.view(), self.clean.view_mut())) }
    }

    /// WASM has no denormal mode to set
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    fn run(&mut self) -> Result<f32, impl std::fmt::Debug> {
        self.df.process(self.noisy.view(), self.clean.view_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady noise with no speech in it comes out much quieter: the point of all this, and a check that the
    /// samples go in and out at the right scale
    #[test]
    fn quiets_noise() {
        let mut denoiser = Denoiser::new().expect("model loads");
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        };
        let (mut before, mut after) = (0.0f32, 0.0f32);
        // A second of noise; the first half lets the model settle
        for i in 0..100 {
            let mut frame: Vec<f32> = (0..FRAME).map(|_| noise() * 0.1 * 32768.0).collect();
            let energy = |f: &[f32]| f.iter().map(|s| s * s).sum::<f32>();
            let input = energy(&frame);
            denoiser.process(&mut frame, 32768.0);
            if i >= 50 {
                before += input;
                after += energy(&frame);
            }
        }
        assert!(after < before / 10.0, "noise went from {before} to {after}");
    }
}
