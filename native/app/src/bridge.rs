//! Engine state into the window's properties and models. Runs on the UI thread for every state the engine hands
//! over; models are updated in place, and only when their rows really changed, so an open picker isn't rebuilt under
//! the pointer.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use engine::{
    CallStats, CameraAccess, ChatEvent, Device, Lang, Link, Model as SpeechModel, ModelStatus, Pair, Path, Phase,
    SpeechNotice, State, TranslatorEvent, VideoStats,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::chat::{Chat, local_time};
use crate::megabytes;
use crate::reactions::Reactions;
use crate::ui::{AppPhase, AppWindow, CallLink, CamAccess, DeviceItem, ModelState, StatsView};

/// What the bridge keeps on the UI thread between states.
pub struct Bridge {
    mics: Rc<VecModel<DeviceItem>>,
    speakers: Rc<VecModel<DeviceItem>>,
    cams: Rc<VecModel<DeviceItem>>,
    screen_sources: Rc<VecModel<DeviceItem>>,
    chat: RefCell<Chat>,
    reactions: RefCell<Reactions>,
    /// Where the translation models are kept
    data_dir: Option<PathBuf>,
    /// What translating subtitles depends on from the last state, for when the switch changes in between
    last: RefCell<(Phase, Lang, ModelStatus)>,
    /// Read by the ticker that redraws the call timer
    pub call_started: Rc<RefCell<Option<Instant>>>,
}

impl Bridge {
    pub fn new(ui: &AppWindow, lang: Lang, data_dir: Option<PathBuf>) -> Self {
        let mics = Rc::new(VecModel::default());
        let speakers = Rc::new(VecModel::default());
        let cams = Rc::new(VecModel::default());
        ui.set_mics(ModelRc::from(mics.clone()));
        ui.set_speakers(ModelRc::from(speakers.clone()));
        ui.set_cams(ModelRc::from(cams.clone()));
        let screen_sources = Rc::new(VecModel::default());
        ui.set_screen_sources(ModelRc::from(screen_sources.clone()));
        Self {
            mics,
            speakers,
            cams,
            screen_sources,
            chat: RefCell::new(Chat::new(ui, lang)),
            reactions: RefCell::new(Reactions::new(ui)),
            data_dir,
            last: RefCell::new((Phase::Start, lang, ModelStatus::Missing)),
            call_started: Rc::default(),
        }
    }

    pub fn chat(&self, ui: &AppWindow, event: ChatEvent) {
        if let ChatEvent::Reaction { emoji, mine } = event {
            self.reactions.borrow_mut().add(emoji, mine);
            return;
        }
        self.chat.borrow_mut().event(ui, event);
    }

    pub fn translated(&self, pair: Pair, event: TranslatorEvent) {
        self.chat.borrow_mut().translation_event(pair, event);
    }

    /// Translates subtitles here while a call is up with them on and the model downloaded
    pub fn sync_translation(&self, ui: &AppWindow) {
        let (phase, lang, model) = *self.last.borrow();
        let up = matches!(phase, Phase::Starting | Phase::Waiting | Phase::InCall);
        let want = (up && ui.get_subtitles_on() && model == ModelStatus::Ready).then(|| Pair::into(lang));
        let weak = ui.as_weak();
        self.chat.borrow_mut().sync_translation(want, self.data_dir.as_deref(), move |pair, event| {
            let _ = weak.upgrade_in_event_loop(move |_| crate::with_bridge(|bridge| bridge.translated(pair, event)));
        });
    }

    pub fn render(&self, ui: &AppWindow, state: &State) {
        *self.call_started.borrow_mut() = state.call_started;
        sync_devices(&self.mics, &state.mics);
        sync_devices(&self.speakers, &state.speakers);
        sync_devices(&self.cams, &state.cams);
        sync_devices(&self.screen_sources, state.screen_picker.as_deref().unwrap_or_default());

        ui.set_phase(match state.phase {
            Phase::Start => AppPhase::Start,
            Phase::Setup => AppPhase::Setup,
            Phase::Starting => AppPhase::Starting,
            Phase::Waiting => AppPhase::Waiting,
            Phase::InCall => AppPhase::InCall,
            Phase::Full => AppPhase::Full,
            Phase::Ended => AppPhase::Ended,
        });
        ui.set_lang(crate::lang_code(state.lang).into());
        self.chat.borrow_mut().set_lang(state.lang);
        ui.set_room_url(state.room_url.clone().unwrap_or_default().into());
        ui.set_reconnecting(state.reconnecting);
        ui.set_link(match state.link {
            None => CallLink::None,
            Some(Link::Connecting) => CallLink::Connecting,
            Some(Link::Connected) => CallLink::Connected,
            Some(Link::Unstable) => CallLink::Unstable,
            Some(Link::Failed) => CallLink::Failed,
        });

        ui.set_mic_on(state.mic_on);
        ui.set_has_mic(state.has_mic);
        // Nothing picked means the system default, which the OS lists first
        let mic = picked(&state.mics, state.mic_id.as_ref());
        ui.set_mic_id(mic.map(|d| d.id.as_str()).unwrap_or_default().into());
        ui.set_mic_label(mic.map(|d| d.label.as_str()).unwrap_or_default().into());
        let speaker = picked(&state.speakers, state.speaker_id.as_ref());
        ui.set_speaker_id(speaker.map(|d| d.id.as_str()).unwrap_or_default().into());
        ui.set_speaker_label(speaker.map(|d| d.label.as_str()).unwrap_or_default().into());
        ui.set_speaker_off(state.speaker_off);
        ui.set_peer_speaker_off(state.remote_speaker_off);

        ui.set_cam_on(state.cam_on);
        ui.set_cam_access(match state.cam_access {
            CameraAccess::Pending => CamAccess::Pending,
            CameraAccess::Granted => CamAccess::Granted,
            CameraAccess::Denied => CamAccess::Denied,
            CameraAccess::Missing => CamAccess::Missing,
        });
        let cam = picked(&state.cams, state.cam_id.as_ref());
        ui.set_cam_id(cam.map(|d| d.id.as_str()).unwrap_or_default().into());
        ui.set_cam_label(cam.map(|d| d.label.as_str()).unwrap_or_default().into());
        ui.set_portrait(state.portrait);
        ui.set_peer_cam_on(state.remote_cam_on);
        ui.set_remote_screen(state.remote_screen);
        ui.set_sharing(state.sharing);
        ui.set_force_relay(state.force_relay);
        ui.set_stats(stats_view(state.stats.as_ref(), state.trying_direct));
        ui.set_transcribe_on(state.transcribe_on);
        let (notice, until) = match state.speech_notice {
            None => (0, String::new()),
            Some(SpeechNotice::Unavailable) => (1, String::new()),
            Some(SpeechNotice::Paused { until_ms }) => (2, local_time(until_ms)),
        };
        ui.set_speech_notice(notice);
        ui.set_speech_paused_until(until.into());
        ui.set_peer_screenshot(state.peer_screenshot);
        let (model, progress) = model_state(state.model);
        ui.set_model_state(model);
        ui.set_model_progress(progress);
        ui.set_on_device(state.on_device);
        let (translation, progress) = model_state(state.translation);
        ui.set_translation_state(translation);
        ui.set_translation_progress(progress);
        let translation_size = Pair::into(state.lang).size();
        ui.set_translation_size(megabytes(translation_size).into());
        let (live, progress) = model_state(state.live);
        ui.set_live_state(live);
        ui.set_live_progress(progress);
        let (pack, progress) =
            pack_state(&[(state.live, SpeechModel::Live.size()), (state.translation, translation_size)]);
        ui.set_pack_state(pack);
        ui.set_pack_progress(progress);
        ui.set_pack_size(megabytes(SpeechModel::Live.size() + translation_size).into());
        *self.last.borrow_mut() = (state.phase, state.lang, state.translation);
        self.sync_translation(ui);
    }
}

fn model_state(status: ModelStatus) -> (ModelState, i32) {
    match status {
        ModelStatus::Missing => (ModelState::Missing, 0),
        ModelStatus::Downloading(percent) => (ModelState::Downloading, i32::from(percent)),
        ModelStatus::Ready => (ModelState::Ready, 100),
        ModelStatus::Failed => (ModelState::Failed, 0),
    }
}

/// Downloads asked for together, with their sizes, as one: ready once all are, and while any downloads, how much of
/// the whole is in
fn pack_state(parts: &[(ModelStatus, u64)]) -> (ModelState, i32) {
    if parts.iter().all(|(s, _)| *s == ModelStatus::Ready) {
        return (ModelState::Ready, 100);
    }
    if parts.iter().any(|(s, _)| matches!(s, ModelStatus::Downloading(_))) {
        let total: u64 = parts.iter().map(|(_, size)| size).sum();
        let done: u64 = parts
            .iter()
            .map(|(s, size)| match s {
                ModelStatus::Ready => size * 100,
                ModelStatus::Downloading(percent) => size * u64::from(*percent),
                _ => 0,
            })
            .sum();
        let percent = done.checked_div(total).unwrap_or(0);
        return (ModelState::Downloading, i32::try_from(percent).unwrap_or(100));
    }
    if parts.iter().any(|(s, _)| *s == ModelStatus::Failed) {
        return (ModelState::Failed, 0);
    }
    (ModelState::Missing, 0)
}

fn picked<'a>(devices: &'a [Device], id: Option<&String>) -> Option<&'a Device> {
    id.and_then(|id| devices.iter().find(|d| &d.id == id)).or(devices.first())
}

fn sync_devices(model: &VecModel<DeviceItem>, devices: &[Device]) {
    let same = model.row_count() == devices.len()
        && model.iter().zip(devices).all(|(row, d)| row.id == d.id.as_str() && row.label == d.label.as_str());
    if !same {
        model.set_vec(
            devices
                .iter()
                .map(|d| DeviceItem { id: SharedString::from(&d.id), label: SharedString::from(&d.label) })
                .collect::<Vec<_>>(),
        );
    }
}

/// The web's StatsOverlay text, "—" where there's nothing yet.
fn stats_view(stats: Option<&CallStats>, trying_direct: bool) -> StatsView {
    let Some(s) = stats else {
        return StatsView { rtt_level: -1, ..empty_stats() };
    };
    let path = match s.path {
        Path::Unknown => "—".to_owned(),
        Path::Relay => format!("TURN relay ({})", s.relay_protocol.or(s.protocol.as_deref()).unwrap_or("?")),
        Path::Direct => format!(
            "Direct P2P ({} ↔ {}, {})",
            s.local_type.unwrap_or("?"),
            s.remote_type.unwrap_or("?"),
            s.protocol.as_deref().unwrap_or("?")
        ),
    };
    let path = if trying_direct { format!("{path} · trying direct…") } else { path };
    StatsView {
        path: path.into(),
        rtt: fmt(s.rtt_ms, "ms", 0).into(),
        rtt_level: match s.rtt_ms {
            None => -1,
            Some(rtt) if rtt < 150.0 => 0,
            Some(rtt) if rtt < 300.0 => 1,
            Some(_) => 2,
        },
        loss: format!("{} / {}", fmt(s.loss_pct, "", 1), fmt(s.loss_out_pct, "%", 1)).into(),
        jitter: fmt(s.jitter_ms, "ms", 0).into(),
        rates: format!("{} / {}", fmt(Some(s.in_kbps), "", 0), fmt(Some(s.out_kbps), "kbps", 0)).into(),
        budget: fmt(s.send_budget_kbps, "kbps", 0).into(),
        video_in: fmt_video(s.video_in.as_ref()).into(),
        video_out: fmt_video(s.video_out.as_ref()).into(),
        limited_by: s.video_out.as_ref().and_then(|v| v.limited_by).unwrap_or_default().into(),
        convert: format!("{:.1} ms", s.convert_ms).into(),
        camera: format!("{:.1} ms", s.camera_ms).into(),
    }
}

fn empty_stats() -> StatsView {
    let dash = SharedString::from("—");
    StatsView {
        path: dash.clone(),
        rtt: dash.clone(),
        rtt_level: -1,
        loss: dash.clone(),
        jitter: dash.clone(),
        rates: dash.clone(),
        budget: dash.clone(),
        video_in: dash.clone(),
        video_out: dash.clone(),
        limited_by: SharedString::default(),
        convert: dash.clone(),
        camera: dash,
    }
}

fn fmt_video(v: Option<&VideoStats>) -> String {
    match v {
        Some(v) if v.width > 0 => {
            let size = format!("{}×{} @ {}", v.width, v.height, fmt(Some(v.fps), "fps", 0));
            match (&v.codec, &v.implementation) {
                (Some(codec), Some(by)) => format!("{size} · {codec} ({by})"),
                (Some(codec), None) => format!("{size} · {codec}"),
                _ => size,
            }
        }
        _ => "—".to_owned(),
    }
}

fn fmt(value: Option<f64>, unit: &str, digits: usize) -> String {
    match value {
        None => "—".to_owned(),
        Some(v) if unit.is_empty() => format!("{v:.digits$}"),
        Some(v) => format!("{v:.digits$} {unit}"),
    }
}

/// 4:05, or 1:04:05 past an hour, like the web's call timer.
pub fn format_duration(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}
