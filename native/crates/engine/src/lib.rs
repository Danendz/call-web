//! Everything in the app that isn't UI. The UI sends [`Command`]s and renders the [`State`] snapshots it gets
//! back, like the web's `CallController`.

mod camera;
mod controller;
mod download;
pub mod friends;
mod level;
mod media;
mod memory;
mod mic;
mod peer;
mod remote;
pub mod room;
mod screen;
mod screen_audio;
mod signaling;
mod speech;
mod stats;
mod translate;
mod video;
mod ws;

pub use camera::CameraAccess;
pub use controller::{ChatEvent, Command, Engine, Link, Phase, State};
pub use media::Device;
pub use protocol::{
    ApiErrorCode, CallEndReason, ChatMessage, FriendState, FriendsLists, FullReason, Lang, MAX_CHAT_LENGTH,
    MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH, MissedCall, OpenRoom, Presence, Reaction, TranslatedText, Translation,
};
pub use speech::{LiveModel, Model, ModelStatus, SpeechNotice, Utterance};
pub use stats::{CallStats, Path, VideoStats};
pub use translate::{Pair, Translator, TranslatorEvent};
pub use video::{Chroma, Feed, Frame, Planes, VideoSink};

/// Hands WebRTC the JavaVM and app context on Android. Call it on a Java thread, before the engine starts: WebRTC
/// finds its Java classes through that thread's class loader.
#[cfg(target_os = "android")]
pub use libwebrtc::android::initialize_android_context;

/// The camera, which only the app's Java side can open
#[cfg(target_os = "android")]
pub use camera::android as android_camera;
