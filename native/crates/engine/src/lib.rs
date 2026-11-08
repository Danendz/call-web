//! Everything in the app that isn't UI. The UI sends [`Command`]s and renders the [`State`] snapshots it gets
//! back, like the web's `CallController`.

mod camera;
mod controller;
mod level;
mod media;
mod mic;
mod peer;
mod remote;
pub mod room;
mod screen;
mod screen_audio;
mod signaling;
mod speech;
mod stats;
mod video;

pub use camera::CameraAccess;
pub use controller::{ChatEvent, Command, Engine, Link, Phase, State};
pub use media::Device;
pub use protocol::{ChatMessage, Lang, MAX_CHAT_LENGTH, TranslatedText, Translation};
pub use speech::{MODEL_SIZE, ModelStatus, SpeechNotice};
pub use stats::{CallStats, Path, VideoStats};
pub use video::{Feed, Frame, VideoSink};

/// Hands WebRTC the JavaVM and app context on Android. Call it on a Java thread, before the engine starts: WebRTC
/// finds its Java classes through that thread's class loader.
#[cfg(target_os = "android")]
pub use libwebrtc::android::initialize_android_context;

/// The camera, which only the app's Java side can open
#[cfg(target_os = "android")]
pub use camera::android as android_camera;
