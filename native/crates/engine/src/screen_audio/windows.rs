//! The shared screen's sound on Windows: process loopback of everything playing except this app's process tree.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};

use super::{CHANNELS, Chunker, SAMPLE_RATE};

pub(super) struct Running {
    stop: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) fn start(chunker: Chunker) -> Option<Running> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let spawned = thread::Builder::new().name("screen-audio".into()).spawn(move || {
        if let Err(e) = capture(chunker, &stopped) {
            tracing::warn!("no screen audio: {e}");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("couldn't start screen audio: {e}");
        return None;
    }
    Some(Running { stop })
}

fn capture(mut chunker: Chunker, stopped: &AtomicBool) -> Result<(), Box<dyn std::error::Error>> {
    // COM for this thread only, which no UI shares
    wasapi::initialize_mta().ok()?;
    let format = WaveFormat::new(32, 32, SampleType::Float, SAMPLE_RATE as usize, CHANNELS as usize, None);
    // Excluding our process tree: this app plays the other person, who must not hear themselves
    let mut client = AudioClient::new_application_loopback_client(std::process::id(), false)?;
    client.initialize_client(
        &format,
        Direction::Capture,
        StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 },
        None,
    )?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut bytes = VecDeque::new();
    client.start_stream()?;
    while !stopped.load(Ordering::Relaxed) {
        // No event comes while nothing plays; the timeout only lets a stop be noticed
        if event.wait_for_event(200).is_err() {
            continue;
        }
        capture.read_from_device_to_deque(&mut bytes)?;
        let whole = bytes.len() - bytes.len() % size_of::<f32>();
        let samples: Vec<f32> = bytes
            .drain(..whole)
            .collect::<Vec<u8>>()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        chunker.push(samples);
    }
    let _ = client.stop_stream();
    Ok(())
}
