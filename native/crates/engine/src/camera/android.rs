//! Android cameras. Only Java can open them (Camera2), so the app hands the engine a [`Backend`] that does, and
//! passes each captured frame back to [`frame`] from the camera's thread.

use std::sync::{Arc, Mutex, OnceLock};

use super::convert::{Crop, Yuv420, timed, yuv420_to_i420};
use super::{CameraAccess, Output};
use crate::media::Device;

/// What the app's Java side does for the camera.
pub trait Backend: Send + Sync {
    /// Asks for the camera permission if it isn't granted; the answer comes to [`access`].
    fn request_access(&self);
    /// Labels are `@front`, `@back` or `@external`, which the UI names in the app's language.
    fn list(&self) -> Vec<Device>;
    /// Opens the camera with this id, or the first one, at about 720p and 30 fps.
    fn start(&self, id: Option<&str>) -> Result<(), String>;
    fn stop(&self);
}

static BACKEND: OnceLock<Box<dyn Backend>> = OnceLock::new();
type Answer = Box<dyn FnOnce(CameraAccess) + Send>;
static WAITING: Mutex<Option<Answer>> = Mutex::new(None);
/// The running camera's output; None while it's stopped, so a late frame goes nowhere
static OUTPUT: Mutex<Option<Arc<Output>>> = Mutex::new(None);

/// Set once at startup, before the engine starts.
pub fn set_backend(backend: Box<dyn Backend>) {
    let _ = BACKEND.set(backend);
}

/// The answer to [`Backend::request_access`].
pub fn access(granted: bool) {
    if let Some(done) = WAITING.lock().ok().and_then(|mut w| w.take()) {
        done(if granted { CameraAccess::Granted } else { CameraAccess::Denied });
    }
}

/// A camera frame as Android's YUV_420_888 planes, `width` x `height` as the sensor sees it, and the clockwise turn
/// that makes it upright on the screen.
pub struct Planes<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub uv_stride: usize,
    pub uv_pixel_stride: usize,
    pub width: u32,
    pub height: u32,
    pub rotation: u32,
}

pub fn frame(planes: &Planes<'_>) {
    let Some(output) = OUTPUT.lock().ok().and_then(|o| o.clone()) else { return };
    // The crop is taken before turning: a sideways sensor's 16:9 picture is already portrait once upright
    let sideways = planes.rotation % 180 == 90;
    let crop = Crop::new(planes.width, planes.height, output.portrait() != sideways);
    let src = Yuv420 {
        y: planes.y,
        y_stride: planes.y_stride,
        u: planes.u,
        v: planes.v,
        uv_stride: planes.uv_stride,
        uv_pixel_stride: planes.uv_pixel_stride,
    };
    if let Some(buffer) = timed(|| yuv420_to_i420(&src, crop, planes.rotation)) {
        output.deliver(buffer);
    }
}

pub(super) fn request_access(done: impl FnOnce(CameraAccess) + Send + 'static) {
    let Some(backend) = BACKEND.get() else { return done(CameraAccess::Missing) };
    if let Ok(mut waiting) = WAITING.lock() {
        *waiting = Some(Box::new(done));
    }
    backend.request_access();
}

pub(super) fn list() -> Vec<Device> {
    BACKEND.get().map(|b| b.list()).unwrap_or_default()
}

pub(super) struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(backend) = BACKEND.get() {
            backend.stop();
        }
        if let Ok(mut current) = OUTPUT.lock() {
            *current = None;
        }
    }
}

pub(super) fn start(id: Option<&str>, output: Arc<Output>) -> Result<Running, String> {
    let backend = BACKEND.get().ok_or("no camera backend")?;
    if let Ok(mut current) = OUTPUT.lock() {
        *current = Some(output);
    }
    let running = Running;
    backend.start(id)?;
    // A phone's encoder keeps up with 720p at 30, and its battery with little more
    super::set_fps(30);
    Ok(running)
}
