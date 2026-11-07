//! Everything in the app that isn't UI. The UI sends [`Command`]s and renders the [`State`] snapshots it gets
//! back, like the web's `CallController`.

mod controller;
pub mod room;
mod signaling;

pub use controller::{Command, Engine, Phase, State};
pub use protocol::Lang;
