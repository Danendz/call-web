//! The shared screen's sound on macOS: a ScreenCaptureKit stream of the main display that keeps only the audio,
//! leaving out this app's own. Finding the display is asynchronous, so the stream starts once it's found, unless the
//! share has stopped by then.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, available, define_class, msg_send};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType,
};

use super::{CHANNELS, Chunker, SAMPLE_RATE};

struct Stream {
    stream: Retained<SCStream>,
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
}

#[derive(Default)]
struct State {
    stopped: bool,
    stream: Option<Stream>,
}

// SAFETY: the stream is only started and stopped through the lock, and SCStream may be stopped from any thread; the
// output's state is a Mutex'd Chunker
unsafe impl Send for State {}

pub(super) struct Running {
    state: Arc<Mutex<State>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else { return };
        state.stopped = true;
        if let Some(stream) = state.stream.take() {
            // SAFETY: stopping a stream we started; its output and queue live until this returns
            unsafe { stream.stream.stopCaptureWithCompletionHandler(None) };
        }
    }
}

pub(super) fn start(chunker: Chunker) -> Option<Running> {
    // Audio capture came to ScreenCaptureKit in macOS 13
    if !available!(macos = 13.0) {
        return None;
    }
    let state = Arc::new(Mutex::new(State::default()));
    let found = state.clone();
    let chunker = Mutex::new(Some(chunker));
    let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        // SAFETY: the content is valid for the length of the call, or null with an error
        let Some(content) = (unsafe { content.as_ref() }) else {
            // SAFETY: as above
            let reason = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
            tracing::warn!("no screen audio, can't list displays: {reason}");
            return;
        };
        let Some(chunker) = chunker.lock().ok().and_then(|mut c| c.take()) else { return };
        let Ok(mut state) = found.lock() else { return };
        if state.stopped {
            return;
        }
        state.stream = open(content, chunker);
    });
    // SAFETY: the block is retained by ScreenCaptureKit until it has been called. Listing what can be captured asks
    // for Screen Recording permission the first time, if the share itself didn't
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &block,
        );
    }
    Some(Running { state })
}

fn open(content: &SCShareableContent, chunker: Chunker) -> Option<Stream> {
    // SAFETY: ScreenCaptureKit calls with valid arguments; the stream, its output and queue live in `Stream`
    unsafe {
        let display = content.displays().firstObject()?;
        let filter =
            SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new());
        let config = SCStreamConfiguration::new();
        config.setCapturesAudio(true);
        config.setExcludesCurrentProcessAudio(true);
        config.setSampleRate(SAMPLE_RATE as isize);
        config.setChannelCount(CHANNELS as isize);
        // The picture is thrown away, so it's as small and rare as it can be
        config.setWidth(2);
        config.setHeight(2);
        config.setMinimumFrameInterval(CMTime { value: 1, timescale: 1, flags: CMTimeFlags::Valid, epoch: 0 });
        let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None);
        let output = Output::new(chunker);
        let queue = DispatchQueue::new("call-danya.screen-audio", None);
        if let Err(e) = stream.addStreamOutput_type_sampleHandlerQueue_error(
            ProtocolObject::from_ref(&*output),
            SCStreamOutputType::Audio,
            Some(&queue),
        ) {
            tracing::warn!("no screen audio: {}", e.localizedDescription());
            return None;
        }
        let started = RcBlock::new(|error: *mut NSError| {
            // Null on success, otherwise valid for the length of the call (inside the outer `unsafe`)
            if let Some(error) = error.as_ref() {
                tracing::warn!("no screen audio: {}", error.localizedDescription());
            }
        });
        stream.startCaptureWithCompletionHandler(Some(&started));
        Some(Stream { stream, _output: output, _queue: queue })
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Output doesn't implement Drop
    #[unsafe(super(NSObject))]
    #[name = "CallDanyaScreenAudioOutput"]
    #[ivars = Mutex<Chunker>]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind == SCStreamOutputType::Audio
                && let Ok(mut chunker) = self.ivars().lock()
            {
                samples(sample, &mut chunker);
            }
        }
    }
);

impl Output {
    fn new(chunker: Chunker) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Mutex::new(chunker));
        // SAFETY: NSObject's init on a freshly allocated object
        unsafe { msg_send![super(this), init] }
    }
}

/// ScreenCaptureKit's audio: 32-bit float, one plane per channel.
fn samples(sample: &CMSampleBuffer, chunker: &mut Chunker) {
    // SAFETY: the sample buffer is valid for the length of the delegate call, and exactly `len` bytes are copied
    // into a buffer of that size
    unsafe {
        let frames = usize::try_from(sample.num_samples()).unwrap_or(0);
        let Some(block) = sample.data_buffer() else { return };
        let len = block.data_length();
        if frames == 0 || len != frames * CHANNELS as usize * size_of::<f32>() {
            return;
        }
        let mut planes = vec![0f32; frames * CHANNELS as usize];
        let Some(dest) = NonNull::new(planes.as_mut_ptr().cast()) else { return };
        if block.copy_data_bytes(0, len, dest) != 0 {
            return;
        }
        let (left, right) = planes.split_at(frames);
        chunker.push(left.iter().zip(right).flat_map(|(l, r)| [*l, *r]));
    }
}
