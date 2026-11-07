//! Engine state into the window's properties and models. Runs on the UI thread for every state the engine hands
//! over; models are updated in place, and only when their rows really changed, so an open picker isn't rebuilt under
//! the pointer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use engine::{CallStats, CameraAccess, Device, Link, Path, Phase, State, VideoStats};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::ui::{AppPhase, AppWindow, CallLink, CamAccess, DeviceItem, StatsView};

/// What the bridge keeps on the UI thread between states.
pub struct Bridge {
    mics: Rc<VecModel<DeviceItem>>,
    cams: Rc<VecModel<DeviceItem>>,
    /// Read by the ticker that redraws the call timer
    pub call_started: Rc<RefCell<Option<Instant>>>,
}

impl Bridge {
    pub fn new(ui: &AppWindow) -> Self {
        let mics = Rc::new(VecModel::default());
        let cams = Rc::new(VecModel::default());
        ui.set_mics(ModelRc::from(mics.clone()));
        ui.set_cams(ModelRc::from(cams.clone()));
        Self { mics, cams, call_started: Rc::default() }
    }

    pub fn render(&self, ui: &AppWindow, state: &State) {
        *self.call_started.borrow_mut() = state.call_started;
        sync_devices(&self.mics, &state.mics);
        sync_devices(&self.cams, &state.cams);

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
        // Nothing picked means the system default, which the OS lists first
        let mic = picked(&state.mics, state.mic_id.as_ref());
        ui.set_mic_id(mic.map(|d| d.id.as_str()).unwrap_or_default().into());
        ui.set_mic_label(mic.map(|d| d.label.as_str()).unwrap_or_default().into());

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
        ui.set_force_relay(state.force_relay);
        ui.set_stats(stats_view(state.stats.as_ref(), state.trying_direct));
    }
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
        convert: dash,
    }
}

fn fmt_video(v: Option<&VideoStats>) -> String {
    match v {
        Some(v) if v.width > 0 => format!("{}×{} @ {}", v.width, v.height, fmt(Some(v.fps), "fps", 0)),
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
