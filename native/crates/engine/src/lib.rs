//! Everything in the app that isn't UI. The UI sends [`Command`]s and renders the [`State`] snapshots it gets
//! back, like the web's `CallController`.

mod controller;
mod level;
mod media;
mod peer;
pub mod room;
mod signaling;

pub use controller::{Command, Engine, Link, Phase, State};
pub use media::Device;
pub use protocol::Lang;
