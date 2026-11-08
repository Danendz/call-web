//! What's remembered between launches, in a small JSON file in the OS's per-user app data folder.

use std::fs;
use std::path::PathBuf;

use engine::Lang;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    pub lang: Option<Lang>,
    /// The picked mic's id, applied when it's plugged in
    pub mic: Option<String>,
    pub cam: Option<String>,
    /// Send 9:16 video instead of 16:9
    #[serde(default)]
    pub portrait: bool,
    /// Speech to chat, which the web also remembers
    #[serde(default)]
    pub transcribe: bool,
    /// Which corner each video tile sits in: "self", "peer" or "screen" to tl, tr, bl or br
    #[serde(default)]
    pub corners: std::collections::BTreeMap<String, String>,
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Losing a setting isn't worth bothering anyone about: failures are only logged.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&path, serde_json::to_vec_pretty(self)?)
        };
        if let Err(e) = write() {
            tracing::warn!("couldn't save settings: {e}");
        }
    }
}

/// The app's private data folder on Android, which only the activity knows; set once at startup.
#[cfg(target_os = "android")]
pub static ANDROID_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn path() -> Option<PathBuf> {
    Some(ANDROID_DIR.get()?.join("settings.json"))
}

#[cfg(not(target_os = "android"))]
fn path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
    };
    Some(base.join("Call Danya").join("settings.json"))
}

/// The OS language, for the first launch: Chinese if it's any Chinese, otherwise English, as the web does.
pub fn system_lang() -> Lang {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_lowercase().starts_with("zh") => Lang::Zh,
        _ => Lang::En,
    }
}
