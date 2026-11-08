//! The mic level meter on the setup screen, like the web's `MicLevel`. WebRTC only captures during a call, so the
//! meter opens the mic itself, and only while the setup screen is up. Opening it there is also when the OS asks for
//! mic permission, as the browser does on the web's setup screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;

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
            let meter = level.clone();
            // Peak of each buffer, falling off smoothly so the bar doesn't flicker between buffers
            let opened = crate::mic::open(device.as_deref(), move |samples| {
                let peak = samples.iter().map(|s| s.abs()).fold(0.0, f32::max).min(1.0);
                meter.set(peak.max(meter.get() * 0.85));
            });
            match opened {
                Ok(input) => {
                    // Blocks until the Meter is dropped
                    let _ = stopped.recv();
                    drop(input);
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
