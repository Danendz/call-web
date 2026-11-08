//! macOS cameras through AVFoundation. Landscape frames go to WebRTC as the camera's own `CVPixelBuffer`, with no
//! copy, which the hardware encoder takes as is; portrait needs a crop, so those are converted to I420.

use std::sync::Arc;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use libwebrtc::video_frame::native::NativeBuffer;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput,
    AVCaptureSession, AVCaptureSessionPreset1280x720, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use super::convert::{Crop, nv12_to_i420};
use super::{CameraAccess, Output};
use crate::media::Device;

pub(super) fn request_access(done: impl FnOnce(CameraAccess) + Send + 'static) {
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return done(CameraAccess::Missing) };
    // SAFETY: a class method taking a valid media type constant
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(video) };
    match status {
        AVAuthorizationStatus::Authorized => done(CameraAccess::Granted),
        AVAuthorizationStatus::NotDetermined => {
            // The block is Fn but must run `done` once; AVFoundation calls it exactly once
            let done = std::sync::Mutex::new(Some(done));
            let block = RcBlock::new(move |granted: Bool| {
                if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                    done(if granted.as_bool() { CameraAccess::Granted } else { CameraAccess::Denied });
                }
            });
            // SAFETY: the block is retained by AVFoundation until it has been called
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &block) };
        }
        _ => done(CameraAccess::Denied),
    }
}

pub(super) fn list() -> Vec<Device> {
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return Vec::new() };
    // The discovery session would need device types that only exist on newer macOS; this lists the same cameras
    // SAFETY: a class method taking a valid media type constant
    #[allow(deprecated)]
    let devices = unsafe { AVCaptureDevice::devicesWithMediaType(video) };
    devices
        .iter()
        // SAFETY: plain property getters on devices AVFoundation just handed us
        .map(|d| unsafe { Device { id: d.uniqueID().to_string(), label: d.localizedName().to_string() } })
        .collect()
}

pub(super) struct Running {
    session: Retained<AVCaptureSession>,
    _delegate: Retained<Delegate>,
    _queue: dispatch2::DispatchRetained<DispatchQueue>,
}

// SAFETY: the session is only stopped from here, and AVCaptureSession's start and stop may be called from any
// thread; the delegate's state is an Arc'd Output that is Send + Sync
unsafe impl Send for Running {}

impl Drop for Running {
    fn drop(&mut self) {
        // SAFETY: stopping a session we started; blocks until frames stop arriving
        unsafe { self.session.stopRunning() };
    }
}

pub(super) fn start(id: Option<&str>, output: Arc<Output>) -> Result<Running, String> {
    let video = unsafe { AVMediaTypeVideo }.ok_or("no video media type")?;
    // SAFETY: AVFoundation calls with valid arguments; the objects live in `Running` for as long as the session runs
    unsafe {
        let device = id
            .and_then(|id| AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(id)))
            .or_else(|| AVCaptureDevice::defaultDeviceWithMediaType(video))
            .ok_or("no camera")?;
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
            .map_err(|e| format!("can't open the camera: {}", e.localizedDescription()))?;

        let session = AVCaptureSession::new();
        session.beginConfiguration();
        if session.canSetSessionPreset(AVCaptureSessionPreset1280x720) {
            session.setSessionPreset(AVCaptureSessionPreset1280x720);
        }
        if !session.canAddInput(&input) {
            session.commitConfiguration();
            return Err("the camera can't be added to a capture session".into());
        }
        session.addInput(&input);

        let video_output = AVCaptureVideoDataOutput::new();
        // NV12, what WebRTC's encoders take without converting; the value of kCVPixelBufferPixelFormatTypeKey
        let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
        let settings = NSDictionary::<NSString, AnyObject>::from_slices(
            &[&*NSString::from_str("PixelFormatType")],
            &[format.as_ref()],
        );
        video_output.setVideoSettings(Some(&settings));
        // A late frame is dropped, never queued
        video_output.setAlwaysDiscardsLateVideoFrames(true);
        let delegate = Delegate::new(output);
        let queue = DispatchQueue::new("call-danya.camera", None);
        video_output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), Some(&queue));
        if !session.canAddOutput(&video_output) {
            session.commitConfiguration();
            return Err("the camera's output can't be added".into());
        }
        session.addOutput(&video_output);
        session.commitConfiguration();
        session.startRunning();
        Ok(Running { session, _delegate: delegate, _queue: queue })
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Delegate doesn't implement Drop
    #[unsafe(super(NSObject))]
    #[name = "CallDanyaCameraDelegate"]
    #[ivars = Arc<Output>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for Delegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output(&self, _output: &AVCaptureOutput, sample: &CMSampleBuffer, _connection: &AVCaptureConnection) {
            frame(self.ivars(), sample);
        }
    }
);

impl Delegate {
    fn new(output: Arc<Output>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(output);
        // SAFETY: NSObject's init on a freshly allocated object
        unsafe { msg_send![super(this), init] }
    }
}

fn frame(output: &Output, sample: &CMSampleBuffer) {
    // SAFETY: the sample buffer is valid for the length of the delegate call
    let Some(image) = (unsafe { sample.image_buffer() }) else { return };
    if !output.portrait() {
        // SAFETY: a valid CVPixelBuffer. The wrapper releases the reference it's given (despite its docs saying it
        // leaves the count alone), so it gets ours; passing a borrowed one over-released every frame and crashed
        let native = unsafe { NativeBuffer::from_cv_pixel_buffer(CFRetained::into_raw(image).as_ptr().cast()) };
        output.deliver(native);
        return;
    }
    let crop = Crop::new(CVPixelBufferGetWidth(&image) as u32, CVPixelBufferGetHeight(&image) as u32, true);
    // SAFETY: read-only lock; the plane pointers and sizes are only used before the matching unlock
    let i420 = unsafe {
        if CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return;
        }
        let plane = |i: usize, rows: usize| {
            let base = CVPixelBufferGetBaseAddressOfPlane(&image, i).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(&image, i);
            let bytes = if base.is_null() { &[][..] } else { std::slice::from_raw_parts(base, stride * rows) };
            (bytes, stride as u32)
        };
        let height = CVPixelBufferGetHeight(&image);
        let (y, y_stride) = plane(0, height);
        let (uv, uv_stride) = plane(1, height.div_ceil(2));
        let i420 = nv12_to_i420(y, y_stride, uv, uv_stride, crop);
        CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
        i420
    };
    if let Some(i420) = i420 {
        output.deliver(i420);
    }
}
