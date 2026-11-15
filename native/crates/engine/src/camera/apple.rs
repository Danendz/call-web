//! macOS and iOS cameras through AVFoundation. Frames go to WebRTC as the camera's own `CVPixelBuffer`, with no copy,
//! which the hardware encoder takes as is, unless they need a crop to the other shape, which converts them to I420.
//! A phone's camera gives sideways frames when it's held upright: they go as they are with the turn that makes them
//! upright, as on Android.

use std::sync::Arc;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use libwebrtc::video_frame::VideoRotation;
use libwebrtc::video_frame::native::NativeBuffer;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput,
    AVCaptureSession, AVCaptureSessionPreset1280x720, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
#[cfg(target_os = "ios")]
use objc2_av_foundation::{
    AVCaptureDeviceDiscoverySession, AVCaptureDevicePosition, AVCaptureDeviceRotationCoordinator,
    AVCaptureDeviceTypeBuiltInWideAngleCamera,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMSampleBuffer;
#[cfg(target_os = "macos")]
use objc2_core_media::{CMTime, CMTimeFlags, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::{
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use super::convert::{Crop, nv12_to_i420, timed};
use super::{CameraAccess, Output};
#[cfg(target_os = "macos")]
use super::{HEIGHT, MAX_FPS, WIDTH};
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

#[cfg(target_os = "macos")]
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

/// The phone's front and back cameras, labeled `@front` and `@back` like Android's, which the UI names in the app's
/// language. The front one comes first: it's the one [`start`] opens when none is picked, and the first one listed is
/// what the UI shows then.
#[cfg(target_os = "ios")]
pub(super) fn list() -> Vec<Device> {
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return Vec::new() };
    // SAFETY: a class method taking valid constants; plain getters on the devices it hands back
    unsafe {
        let types = objc2_foundation::NSArray::from_slice(&[AVCaptureDeviceTypeBuiltInWideAngleCamera]);
        let found = AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
            &types,
            Some(video),
            AVCaptureDevicePosition::Unspecified,
        );
        let mut devices = found.devices().to_vec();
        devices.sort_by_key(|d| d.position() != AVCaptureDevicePosition::Front);
        devices
            .iter()
            .map(|d| Device {
                id: d.uniqueID().to_string(),
                label: if d.position() == AVCaptureDevicePosition::Front { "@front" } else { "@back" }.into(),
            })
            .collect()
    }
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
            .or_else(|| default_device(video))
            .ok_or("no camera")?;
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
            .map_err(|e| format!("can't open the camera: {}", e.localizedDescription()))?;

        let session = AVCaptureSession::new();
        // WebRTC owns the app's audio session for the call; left on, starting the camera would set it up again
        #[cfg(target_os = "ios")]
        session.setAutomaticallyConfiguresApplicationAudioSession(false);
        session.beginConfiguration();
        if session.canSetSessionPreset(AVCaptureSessionPreset1280x720) {
            session.setSessionPreset(AVCaptureSessionPreset1280x720);
        }
        if !session.canAddInput(&input) {
            session.commitConfiguration();
            return Err("the camera can't be added to a capture session".into());
        }
        session.addInput(&input);
        // After the input is added: picking a format overrides the session's preset
        #[cfg(target_os = "macos")]
        super::set_fps(prefer_high_fps(&device));
        // A phone's encoder keeps up with 720p at 30, and its battery with little more
        #[cfg(target_os = "ios")]
        super::set_fps(30);

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
        let delegate = Delegate::new(Ivars {
            output,
            #[cfg(target_os = "ios")]
            turn: AVCaptureDeviceRotationCoordinator::initWithDevice_previewLayer(
                AVCaptureDeviceRotationCoordinator::alloc(),
                &device,
                None,
            ),
        });
        let queue = DispatchQueue::new("call-danya.camera", None);
        video_output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), Some(&queue));
        if !session.canAddOutput(&video_output) {
            session.commitConfiguration();
            return Err("the camera's output can't be added".into());
        }
        session.addOutput(&video_output);
        // iOS mirrors the front camera's frames on its own; they go out as the camera sees, like the back camera's,
        // and only our own view of them is mirrored
        #[cfg(target_os = "ios")]
        if let Some(connection) = video_output.connectionWithMediaType(video)
            && connection.isVideoMirroringSupported()
        {
            tracing::info!("camera frames mirrored by iOS: {}", connection.isVideoMirrored());
            connection.setAutomaticallyAdjustsVideoMirroring(false);
            connection.setVideoMirrored(false);
        }
        session.commitConfiguration();
        session.startRunning();
        Ok(Running { session, _delegate: delegate, _queue: queue })
    }
}

/// The front camera on a phone, the one a call is made with
#[cfg(target_os = "ios")]
unsafe fn default_device(video: &objc2_av_foundation::AVMediaType) -> Option<Retained<AVCaptureDevice>> {
    // SAFETY: a class method taking valid constants
    unsafe {
        let wide = AVCaptureDeviceTypeBuiltInWideAngleCamera;
        AVCaptureDevice::defaultDeviceWithDeviceType_mediaType_position(
            wide,
            Some(video),
            AVCaptureDevicePosition::Front,
        )
        .or_else(|| AVCaptureDevice::defaultDeviceWithMediaType(video))
    }
}

#[cfg(target_os = "macos")]
unsafe fn default_device(video: &objc2_av_foundation::AVMediaType) -> Option<Retained<AVCaptureDevice>> {
    // SAFETY: a class method taking a valid media type constant
    unsafe { AVCaptureDevice::defaultDeviceWithMediaType(video) }
}

/// Switches to the camera's 720p format with the highest frame rate up to [`MAX_FPS`], if it beats the preset's 30;
/// returns the frame rate it runs at. Built-in cameras stop at 30; an iPhone as Continuity Camera gives 60.
///
/// SAFETY: the device is in a session being configured, which is when its format may change
#[cfg(target_os = "macos")]
unsafe fn prefer_high_fps(device: &AVCaptureDevice) -> u32 {
    // SAFETY: plain getters on a device AVFoundation handed us
    let best = unsafe {
        device
            .formats()
            .iter()
            .filter(|f| {
                let size = CMVideoFormatDescriptionGetDimensions(&f.formatDescription());
                (size.width, size.height) == (WIDTH as i32, HEIGHT as i32)
            })
            .map(|f| {
                let top = f.videoSupportedFrameRateRanges().iter().map(|r| r.maxFrameRate()).fold(0.0, f64::max);
                (top.min(f64::from(MAX_FPS)) as u32, f)
            })
            .max_by_key(|(fps, _)| *fps)
    };
    let Some((fps, format)) = best.filter(|(fps, _)| *fps > 30) else { return 30 };
    // SAFETY: the format is one of this device's; the frame duration is within its range, and is set after the
    // format because changing the format resets it
    unsafe {
        if device.lockForConfiguration().is_err() {
            return 30;
        }
        device.setActiveFormat(&format);
        device.setActiveVideoMinFrameDuration(CMTime {
            value: 1,
            timescale: fps as i32,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
        device.unlockForConfiguration();
    }
    fps
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Delegate doesn't implement Drop
    #[unsafe(super(NSObject))]
    #[name = "CallDanyaCameraDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for Delegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            frame(self.ivars(), sample);
        }
    }
);

struct Ivars {
    output: Arc<Output>,
    /// Which way up the phone is held
    #[cfg(target_os = "ios")]
    turn: Retained<AVCaptureDeviceRotationCoordinator>,
}

impl Ivars {
    /// The clockwise turn that makes the camera's picture upright
    fn rotation(&self) -> VideoRotation {
        #[cfg(target_os = "ios")]
        // SAFETY: a plain getter, which AVFoundation keeps up to date from the main thread
        match unsafe { self.turn.videoRotationAngleForHorizonLevelCapture() }.round() as i32 {
            90 => return VideoRotation::VideoRotation90,
            180 => return VideoRotation::VideoRotation180,
            270 => return VideoRotation::VideoRotation270,
            _ => {}
        }
        VideoRotation::VideoRotation0
    }
}

impl Delegate {
    fn new(ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        // SAFETY: NSObject's init on a freshly allocated object
        unsafe { msg_send![super(this), init] }
    }
}

fn frame(ivars: &Ivars, sample: &CMSampleBuffer) {
    let output = &ivars.output;
    // SAFETY: the sample buffer is valid for the length of the delegate call
    let Some(image) = (unsafe { sample.image_buffer() }) else { return };
    let rotation = ivars.rotation();
    // The crop is taken before turning: a sideways 16:9 picture is already portrait once upright
    let sideways = matches!(rotation, VideoRotation::VideoRotation90 | VideoRotation::VideoRotation270);
    if output.portrait() == sideways {
        // SAFETY: a valid CVPixelBuffer. The wrapper releases the reference it's given (despite its docs saying it
        // leaves the count alone), so it gets ours; passing a borrowed one over-released every frame and crashed
        let native = unsafe { NativeBuffer::from_cv_pixel_buffer(CFRetained::into_raw(image).as_ptr().cast()) };
        output.deliver_turned(native, rotation);
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
        let i420 = timed(|| nv12_to_i420(y, y_stride, uv, uv_stride, crop));
        CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
        i420
    };
    if let Some(i420) = i420 {
        output.deliver_turned(i420, rotation);
    }
}
