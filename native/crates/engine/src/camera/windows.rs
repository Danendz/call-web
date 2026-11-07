//! Windows cameras through Media Foundation, via `nokhwa`. Uncompressed formats only (NV12, then YUY2): decoding
//! MJPEG would bring a third copy of libjpeg into the binary.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use nokhwa::Camera;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};

use super::convert::{Crop, nv12_to_i420, yuyv_to_i420};
use super::{HEIGHT, Output, WIDTH};
use crate::media::Device;

const FORMATS: &[FrameFormat] = &[FrameFormat::NV12, FrameFormat::YUYV];

pub(super) fn list() -> Vec<Device> {
    match nokhwa::query(ApiBackend::MediaFoundation) {
        Ok(cameras) => {
            cameras.into_iter().map(|c| Device { id: c.index().as_string(), label: c.human_name() }).collect()
        }
        Err(e) => {
            tracing::info!("no cameras: {e}");
            Vec::new()
        }
    }
}

pub(super) struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The camera is released when its thread ends; the next start must find it free
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn start(id: Option<&str>, output: Arc<Output>) -> Result<Running, String> {
    // nokhwa lists Media Foundation cameras by position
    let index = CameraIndex::Index(id.and_then(|id| id.parse().ok()).unwrap_or(0));
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    // Media Foundation objects stay on the thread that made them, so the camera is opened there; the first frame
    // or the error comes back before this returns
    let (opened, open_result) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("camera".into())
        .spawn(move || {
            let mut camera = match open(index) {
                Ok(camera) => camera,
                Err(e) => {
                    let _ = opened.send(Err(e));
                    return;
                }
            };
            let _ = opened.send(Ok(()));
            while !stopped.load(Ordering::Relaxed) {
                let frame = match camera.frame() {
                    Ok(frame) => frame,
                    Err(e) => {
                        tracing::warn!("camera stopped: {e}");
                        break;
                    }
                };
                let Resolution { width_x: width, height_y: height } = frame.resolution();
                let crop = Crop::new(width, height, output.portrait());
                let data = frame.buffer();
                let i420 = match frame.source_frame_format() {
                    FrameFormat::NV12 => data
                        .split_at_checked((width * height) as usize)
                        .and_then(|(y, uv)| nv12_to_i420(y, width, uv, width, crop)),
                    FrameFormat::YUYV => yuyv_to_i420(data, width * 2, crop),
                    _ => None,
                };
                if let Some(i420) = i420 {
                    output.deliver(i420);
                }
            }
            let _ = camera.stop_stream();
        })
        .map_err(|e| e.to_string())?;
    let running = Running { stop, thread: Some(thread) };
    open_result.recv().map_err(|_| "the camera thread ended".to_owned())??;
    Ok(running)
}

fn open(index: CameraIndex) -> Result<Camera, String> {
    let wanted = CameraFormat::new(Resolution::new(WIDTH, HEIGHT), FrameFormat::NV12, 30);
    let request = RequestedFormat::with_formats(RequestedFormatType::Closest(wanted), FORMATS);
    let mut camera = Camera::with_backend(index, request, ApiBackend::MediaFoundation)
        .or_else(|_| Camera::with_backend(CameraIndex::Index(0), request, ApiBackend::MediaFoundation))
        .map_err(|e| e.to_string())?;
    camera.open_stream().map_err(|e| e.to_string())?;
    Ok(camera)
}
