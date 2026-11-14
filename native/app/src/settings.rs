//! What's remembered between launches, in a small JSON file in the OS's per-user app data folder.

use std::fs;
use std::path::PathBuf;

use engine::Lang;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub lang: Option<Lang>,
    /// The picked mic's id, applied when it's plugged in
    pub mic: Option<String>,
    #[serde(default)]
    pub speaker: Option<String>,
    pub cam: Option<String>,
    /// Send 9:16 video instead of 16:9; a phone is held upright, so it starts in portrait
    #[serde(default = "upright")]
    pub portrait: bool,
    /// Speech to chat, which the web also remembers
    #[serde(default)]
    pub transcribe: bool,
    /// Which corner each video tile sits in: "self", "peer" or "screen" to tl, tr, bl or br
    #[serde(default)]
    pub corners: std::collections::BTreeMap<String, String>,
    /// The phone's guide to its battery and app launch settings has been read
    #[serde(default)]
    pub background_guide_seen: bool,
    /// Speech is recognized on this device once its model is downloaded, instead of on the server
    #[serde(default = "yes")]
    pub on_device_speech: bool,
    /// The question about downloading live subtitles' models was answered
    #[serde(default)]
    pub live_prompt_seen: bool,
    /// "Connection stats" in Settings: the overlay shows in every call
    #[serde(default)]
    pub show_stats: bool,
    /// "Live subtitles" in Settings: the last spoken lines over the video
    #[serde(default = "yes")]
    pub subtitles: bool,
    /// DeepFilterNet on the mic. Off on a phone, where it costs battery and competes with a shared screen
    #[serde(default = "desktop")]
    pub noise_suppression: bool,
    #[serde(default)]
    pub video_codec: VideoCodec,
    /// Languages whose translation model was offered and put off; picking the language again offers it again
    #[serde(default)]
    pub translation_prompt_dismissed: Vec<Lang>,
}

/// "Video codec" in Settings, as the settings file and the UI name it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Vp8,
    /// Also what a name this version doesn't know reads as
    #[default]
    #[serde(other)]
    Auto,
}

impl VideoCodec {
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Vp8 => "vp8",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "h264" => Self::H264,
            "vp8" => Self::Vp8,
            _ => Self::Auto,
        }
    }

    pub fn engine(self) -> engine::VideoCodec {
        match self {
            Self::Auto => engine::VideoCodec::Auto,
            Self::H264 => engine::VideoCodec::H264,
            Self::Vp8 => engine::VideoCodec::Vp8,
        }
    }
}

fn yes() -> bool {
    true
}

fn desktop() -> bool {
    !cfg!(target_os = "android")
}

fn upright() -> bool {
    cfg!(target_os = "android")
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: None,
            mic: None,
            speaker: None,
            cam: None,
            portrait: upright(),
            transcribe: false,
            corners: std::collections::BTreeMap::new(),
            background_guide_seen: false,
            on_device_speech: true,
            live_prompt_seen: false,
            show_stats: false,
            subtitles: true,
            noise_suppression: desktop(),
            video_codec: VideoCodec::Auto,
            translation_prompt_dismissed: Vec::new(),
        }
    }
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

fn path() -> Option<PathBuf> {
    Some(data_dir()?.join("settings.json"))
}

/// The app's own folder, for settings and the speech model.
#[cfg(target_os = "android")]
pub fn data_dir() -> Option<PathBuf> {
    ANDROID_DIR.get().cloned()
}

#[cfg(not(target_os = "android"))]
pub fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
    };
    Some(base.join("Call Danya"))
}

/// The OS language, for the first launch: Chinese if it's any Chinese, otherwise English, as the web does.
pub fn system_lang() -> Lang {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_lowercase().starts_with("zh") => Lang::Zh,
        _ => Lang::En,
    }
}
