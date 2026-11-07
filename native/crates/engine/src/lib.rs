//! Everything in the app that isn't UI. The UI sends [`Command`]s and renders the [`State`] snapshots it gets
//! back, like the web's `CallController`.

mod camera;
mod controller;
mod level;
mod media;
mod mic;
mod peer;
pub mod room;
mod signaling;
mod speech;
mod stats;
mod video;

pub use camera::CameraAccess;
pub use controller::{ChatEvent, Command, Engine, Link, Phase, State};
pub use media::Device;
pub use protocol::{ChatMessage, Lang, MAX_CHAT_LENGTH, TranslatedText, Translation};
pub use speech::SpeechNotice;
pub use stats::{CallStats, Path, VideoStats};
pub use video::{Feed, Frame, VideoSink};
