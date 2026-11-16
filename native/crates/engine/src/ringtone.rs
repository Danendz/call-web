//! A friend's call ringing on desktop: the web's two short tones every few seconds (`ringtone.ts`), made here so
//! there's no file to ship. It plays through the default output for as long as the [`Ringtone`] is held.

use std::f32::consts::TAU;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

const PATTERN_S: f32 = 3.0;
const SECOND_TONE_AT: f32 = 0.45;
const VOLUME: f32 = 0.15;

pub struct Ringtone(#[allow(dead_code, reason = "held to keep the stream playing")] cpal::Stream);

impl Ringtone {
    /// Starts ringing; None (logged) when there's no output to ring on.
    pub fn start() -> Option<Self> {
        match open() {
            Ok(stream) => Some(Self(stream)),
            Err(e) => {
                tracing::info!("no ring tone: {e}");
                None
            }
        }
    }
}

fn open() -> Result<cpal::Stream, String> {
    let device = cpal::default_host().default_output_device().ok_or("no output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config),
        SampleFormat::I16 => build::<i16>(&device, &config),
        SampleFormat::I32 => build::<i32>(&device, &config),
        other => Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels()).max(1);
    let rate = config.sample_rate().max(1) as f32;
    // Counted within one pattern, so the time stays exact however long it rings
    let pattern = (PATTERN_S * rate) as u32;
    let mut n = 0u32;
    let mut logged = false;
    device
        .build_output_stream(
            config.config(),
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    frame.fill(T::from_sample(sample(n as f32 / rate)));
                    n = (n + 1) % pattern.max(1);
                }
            },
            move |e| {
                if !std::mem::replace(&mut logged, true) {
                    tracing::info!("ring tone stream: {e}");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// The ring `t` seconds into its pattern: 880 Hz, then 660 Hz, each fading in for 20 ms and out by 350 ms.
fn sample(t: f32) -> f32 {
    let (at, freq) = if t < SECOND_TONE_AT { (t, 880.0) } else { (t - SECOND_TONE_AT, 660.0) };
    let gain = if at < 0.02 {
        at / 0.02
    } else if at < 0.35 {
        1.0 - (at - 0.02) / 0.33
    } else {
        0.0
    };
    gain * VOLUME * (TAU * freq * at).sin()
}
