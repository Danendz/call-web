//! A moving test pattern in place of a camera (`CALL_FAKE_CAMERA=1`), like Chrome's fake capture device.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use libwebrtc::prelude::I420Buffer;

use super::convert::Crop;
use super::{HEIGHT, Output, WIDTH};
use crate::media::Device;

pub(super) fn device() -> Device {
    Device { id: "fake".into(), label: "Fake camera".into() }
}

pub(super) struct Running {
    stop: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) fn start(output: Arc<Output>) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let spawned = thread::Builder::new().name("fake-camera".into()).spawn(move || {
        let mut tick = 0u32;
        while !stopped.load(Ordering::Relaxed) {
            let crop = Crop::new(WIDTH, HEIGHT, output.portrait());
            output.deliver(pattern(crop.width, crop.height, tick));
            tick = tick.wrapping_add(1);
            thread::sleep(Duration::from_millis(66));
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("couldn't start the fake camera: {e}");
    }
    Running { stop }
}

/// A slowly changing color with a white bar sweeping across, so a frozen picture is easy to spot.
fn pattern(width: u32, height: u32, tick: u32) -> I420Buffer {
    let mut buf = I420Buffer::new(width, height);
    let (sy, su, sv) = buf.strides();
    let (y, u, v) = buf.data_mut();
    let bar = (tick * 8) % width;
    for row in 0..height {
        for col in 0..width {
            let luma = if col.abs_diff(bar) < 12 { 235 } else { 60 + (row * 100 / height) as u8 };
            if let Some(p) = y.get_mut((row * sy + col) as usize) {
                *p = luma;
            }
        }
    }
    let (cu, cv) = (128u8.wrapping_add((tick % 64) as u8), 160u8);
    for row in 0..height.div_ceil(2) {
        for col in 0..width.div_ceil(2) {
            if let Some(p) = u.get_mut((row * su + col) as usize) {
                *p = cu;
            }
            if let Some(p) = v.get_mut((row * sv + col) as usize) {
                *p = cv;
            }
        }
    }
    buf
}
