//! Opening a mic outside WebRTC, for what needs its samples directly: the setup screen's level meter, and speech to
//! chat without WebRTC's platform audio or with `CALL_FAKE_MIC`. In a call, speech to chat reads WebRTC's capture.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// A running capture; dropping it closes the mic. Not `Send` on every platform, so it stays on the thread that
/// opened it.
pub(crate) struct Input {
    _source: Source,
    pub(crate) sample_rate: u32,
}

enum Source {
    Device(#[allow(dead_code, reason = "held to keep the stream running")] cpal::Stream),
    File(#[allow(dead_code, reason = "held to stop the file's thread on drop")] FakeMic),
}

/// Opens the mic WebRTC lists under `name` (the OS's name for it), or the system default. `on_samples` gets mono
/// samples in -1..1, on the audio thread.
pub(crate) fn open(name: Option<&str>, on_samples: impl FnMut(&[f32]) + Send + 'static) -> Result<Input, String> {
    if let Some(path) = std::env::var_os("CALL_FAKE_MIC") {
        return fake(std::path::Path::new(&path), on_samples);
    }
    let host = cpal::default_host();
    let named = name
        .and_then(|name| host.input_devices().ok()?.find(|d| d.description().is_ok_and(|desc| desc.name() == name)));
    if named.is_none()
        && let Some(name) = name
    {
        tracing::info!("no mic named {name:?} outside WebRTC, using the system default");
    }
    let device = named.or_else(|| host.default_input_device()).ok_or("no input device")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, on_samples),
        SampleFormat::I16 => build::<i16>(&device, &config, on_samples),
        SampleFormat::I32 => build::<i32>(&device, &config, on_samples),
        other => Err(format!("unsupported sample format {other:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Input { _source: Source::Device(stream), sample_rate: config.sample_rate() })
}

/// `CALL_FAKE_MIC=<file.wav>` plays a 16-bit mono WAV in a loop, with a pause between rounds, in place of the mic:
/// for testing speech to chat where no mic can be used. `say -o hi.wav --data-format=LEI16@16000 "Hello"` makes one
/// on macOS.
struct FakeMic {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for FakeMic {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn fake(path: &std::path::Path, mut on_samples: impl FnMut(&[f32]) + Send + 'static) -> Result<Input, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("CALL_FAKE_MIC: {e}"))?;
    let (sample_rate, pcm) = wav(&bytes).ok_or("CALL_FAKE_MIC isn't a 16-bit mono WAV")?;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        let ten_ms = (sample_rate / 100) as usize;
        let pause = vec![0.0; ten_ms];
        while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
            for chunk in pcm.chunks(ten_ms).chain(std::iter::repeat_n(pause.as_slice(), 200)) {
                if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                on_samples(chunk);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });
    Ok(Input { _source: Source::File(FakeMic { stop }), sample_rate })
}

/// The rate and samples of a 16-bit mono PCM WAV.
fn wav(bytes: &[u8]) -> Option<(u32, Vec<f32>)> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut rate, mut data) = (12, None, None);
    while at + 8 <= bytes.len() {
        let (id, size) = (bytes.get(at..at + 4)?, u32_at(at + 4)? as usize);
        let body = at + 8;
        match id {
            b"fmt " if u16_at(body)? == 1 && u16_at(body + 2)? == 1 && u16_at(body + 14)? == 16 => {
                rate = u32_at(body + 4);
            }
            b"data" => data = bytes.get(body..(body + size).min(bytes.len())),
            _ => {}
        }
        at = body + size + size % 2;
    }
    let samples = data?.as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0).collect();
    Some((rate?, samples))
}

fn build<T: SizedSample>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut on_samples: impl FnMut(&[f32]) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels()).max(1);
    let mut mono = Vec::new();
    let mut logged = false;
    device
        .build_input_stream(
            config.config(),
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                mono.clear();
                mono.extend(
                    data.chunks(channels)
                        .map(|frame| frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / frame.len() as f32),
                );
                on_samples(&mono);
            },
            move |e| {
                if !std::mem::replace(&mut logged, true) {
                    tracing::info!("mic stream: {e}");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}
