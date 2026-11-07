//! The mic level meter on the setup screen, like the web's `MicLevel`. WebRTC only captures during a call, so the
//! meter opens the mic itself, and only while the setup screen is up. Opening it there is also when the OS asks for
//! mic permission, as the browser does on the web's setup screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};

/// The loudest recent sample, 0..=1 as `f32` bits, read by the UI on its own clock. Shared so the UI never waits
/// on the engine for it.
#[derive(Clone, Default)]
pub struct Level(Arc<AtomicU32>);

impl Level {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Runs until dropped. The stream lives on its own thread: on some platforms it can't move between threads.
pub struct Meter {
    stop: mpsc::Sender<()>,
}

impl Meter {
    /// `device` is the mic's name as WebRTC lists it, which is the OS's name for it; the system default otherwise.
    pub fn start(device: Option<String>, level: Level) -> Self {
        let (stop, stopped) = mpsc::channel();
        let spawned = thread::Builder::new().name("mic-level".into()).spawn(move || {
            let failed = Arc::new(AtomicBool::new(false));
            match open(device.as_deref(), level.clone(), failed) {
                Ok(stream) => {
                    // Blocks until the Meter is dropped
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(e) => tracing::info!("no mic level: {e}"),
            }
            level.set(0.0);
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the mic level thread: {e}");
        }
        Self { stop }
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

fn open(device: Option<&str>, level: Level, failed: Arc<AtomicBool>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let named = device
        .and_then(|name| host.input_devices().ok()?.find(|d| d.description().is_ok_and(|desc| desc.name() == name)));
    let device = named.or_else(|| host.default_input_device()).ok_or("no input device")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let on_error = move |e: cpal::Error| {
        if !failed.swap(true, Ordering::Relaxed) {
            tracing::info!("mic level stream: {e}");
        }
    };
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(stream_config, meter::<f32>(level), on_error, None),
        SampleFormat::I16 => device.build_input_stream(stream_config, meter::<i16>(level), on_error, None),
        SampleFormat::I32 => device.build_input_stream(stream_config, meter::<i32>(level), on_error, None),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Peak of each buffer, falling off smoothly so the bar doesn't flicker between buffers.
fn meter<T: Sample>(level: Level) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    f32: cpal::FromSample<T>,
{
    move |data, _| {
        let peak = data.iter().map(|s| f32::from_sample(*s).abs()).fold(0.0, f32::max).min(1.0);
        level.set(peak.max(level.get() * 0.85));
    }
}
