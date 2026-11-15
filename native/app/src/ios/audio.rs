//! Where the call's sound plays: the speaker, the earpiece, Bluetooth or a wired or USB headset, with the mic coming
//! along, as on Android (`Shim.kt`). WebRTC sets the audio session up for the call in voice-chat mode, which plays at
//! the earpiece, and iOS drops a route picked here whenever the route changes. So at each route change the call is
//! moved where it should be: the route the person picked while it's there, else Bluetooth, else a headset, else the
//! speaker. The call screen shows where iOS actually plays it. The mic list is the same routes seen from their mics:
//! cpal only knows iOS's one default mic, so the phone's own list of mics stands in for it, and picking one moves the
//! call's sound along with it.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionPort, AVAudioSessionPortBluetoothA2DP,
    AVAudioSessionPortBluetoothHFP, AVAudioSessionPortBluetoothLE, AVAudioSessionPortBuiltInMic,
    AVAudioSessionPortBuiltInReceiver, AVAudioSessionPortBuiltInSpeaker, AVAudioSessionPortDescription,
    AVAudioSessionPortHeadphones, AVAudioSessionPortHeadsetMic, AVAudioSessionPortOverride, AVAudioSessionPortUSBAudio,
    AVAudioSessionRouteChangeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};
use objc2_ui_kit::{UIDevice, UIUserInterfaceIdiom};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ui::{AppPhase, AppWindow, DeviceItem};

/// How many times one change may move the call before iOS's own choice is left alone, so a route iOS keeps refusing
/// doesn't loop
const MOVES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Speaker,
    Earpiece,
    Bluetooth,
    Headset,
}

impl Route {
    const ALL: [Route; 4] = [Route::Speaker, Route::Earpiece, Route::Bluetooth, Route::Headset];

    /// The call screen's name for it
    fn id(self) -> &'static str {
        match self {
            Route::Speaker => "speaker",
            Route::Earpiece => "earpiece",
            Route::Bluetooth => "bluetooth",
            Route::Headset => "headset",
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Default)]
struct State {
    ui: Option<slint::Weak<AppWindow>>,
    _observer: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    /// The setup or call screen is up
    active: bool,
    /// The route the person picked on the setup screen or in the call; a headset coming or going no longer moves it
    picked: Option<Route>,
    /// The routes there were at the last change, as [`Route::bit`]s
    available: u8,
    /// Moves left before iOS's choice is kept, see [`MOVES`]
    moves: u8,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Listens for route changes and takes the setup and call screens' picks.
pub fn attach(ui: &AppWindow) {
    ui.on_pick_output(|id| {
        let Some(route) = Route::ALL.into_iter().find(|r| r.id() == id.as_str()) else { return };
        STATE.with_borrow_mut(|state| {
            state.picked = Some(route);
            state.moves = MOVES;
        });
        changed();
    });
    ui.on_pick_mic(|uid| {
        // SAFETY: the shared session lives for the app's whole life
        let session = unsafe { AVAudioSession::sharedInstance() };
        let Some(port) = inputs(&session).into_iter().find(|port| unsafe { port.UID() }.to_string() == uid.as_str())
        else {
            return;
        };
        // SAFETY: reading a port's type has no preconditions
        let route = route_of(&*unsafe { port.portType() }).unwrap_or_else(|| {
            // The phone's own mic, which plays at the speaker or the earpiece, whichever it's at now
            Some(current(&session))
                .flatten()
                .filter(|r| matches!(r, Route::Speaker | Route::Earpiece))
                .unwrap_or(Route::Speaker)
        });
        STATE.with_borrow_mut(|state| {
            state.picked = Some(route);
            state.moves = MOVES;
        });
        changed();
    });
    // Posted on the main queue, where the UI and `STATE` live
    let block = RcBlock::new(|_: NonNull<NSNotification>| changed());
    // SAFETY: the notification name is a valid NSString constant, no object is filtered on, and the block captures
    // nothing that can't move between threads
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            AVAudioSessionRouteChangeNotification,
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    };
    STATE.with_borrow_mut(|state| {
        state.ui = Some(ui.as_weak());
        state._observer = Some(observer);
    });
}

/// Sets the audio session up when the setup screen opens, so its meter has a mic and the route can be picked there,
/// keeps that pick for the call, and turns the session off after it; called on the UI's clock.
pub fn sync(ui: &AppWindow) {
    let active = matches!(ui.get_phase(), AppPhase::Setup | AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall);
    let changed_now = STATE.with_borrow_mut(|state| {
        if state.active == active {
            return false;
        }
        state.active = active;
        state.picked = None;
        state.available = 0;
        true
    });
    if !changed_now {
        return;
    }
    // Both wait on the audio hardware; iOS posts a route change once the session is set
    std::thread::spawn(
        move || if active { engine::audio_session::prepare() } else { engine::audio_session::release() },
    );
    changed();
}

/// Moves the call where it should be if it isn't there, and shows where it is.
fn changed() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: the shared session lives for the app's whole life
    let session = unsafe { AVAudioSession::sharedInstance() };
    // Set by `audio_session::prepare` and then WebRTC; until then there's nothing to route
    // SAFETY: reading the session's category has no preconditions, and the constant is a valid NSString
    let calling = unsafe { Some(&*session.category()) == AVAudioSessionCategoryPlayAndRecord };
    let active = STATE.with_borrow(|state| state.active) && calling;
    let available = if active { available(&session, mtm) } else { 0 };
    let current = if active { current(&session) } else { None };
    let mv = STATE.with_borrow_mut(|state| {
        if available != state.available {
            state.available = available;
            state.moves = MOVES;
        }
        let wanted = state
            .picked
            .filter(|r| available & r.bit() != 0)
            .or_else(|| [Route::Bluetooth, Route::Headset].into_iter().find(|r| available & r.bit() != 0))
            .unwrap_or(Route::Speaker);
        (active && current != Some(wanted) && state.moves > 0).then(|| {
            state.moves -= 1;
            wanted
        })
    });
    if let Some(route) = mv {
        // Changing the route waits on the audio hardware, so not on the UI thread; iOS then posts a route change
        std::thread::spawn(move || route_to(route));
    }

    UIDevice::currentDevice(mtm).setProximityMonitoringEnabled(current == Some(Route::Earpiece));
    let Some(ui) = STATE.with_borrow(|state| state.ui.as_ref().and_then(slint::Weak::upgrade)) else { return };
    // One output is no choice
    let outputs: Vec<DeviceItem> = if available.count_ones() < 2 {
        Vec::new()
    } else {
        Route::ALL
            .into_iter()
            .filter(|r| available & r.bit() != 0)
            .map(|r| DeviceItem { id: r.id().into(), label: format!("@{}", r.id()).into() })
            .collect()
    };
    ui.set_outputs(ModelRc::new(VecModel::from(outputs)));
    ui.set_output(current.map(Route::id).unwrap_or_default().into());

    let mics: Vec<DeviceItem> =
        if active { inputs(&session).iter().map(|port| mic_item(port)).collect() } else { Vec::new() };
    // SAFETY: reading the session's route has no preconditions
    let mic =
        active.then(|| unsafe { session.currentRoute().inputs().firstObject() }).flatten().map(|port| mic_item(&port));
    ui.set_mic_id(mic.as_ref().map(|m| m.id.clone()).unwrap_or_default());
    ui.set_mic_label(mic.map(|m| m.label).unwrap_or_default());
    ui.set_mics(ModelRc::new(VecModel::from(mics)));
}

/// The mics iOS can record the call from
fn inputs(session: &AVAudioSession) -> Vec<Retained<AVAudioSessionPortDescription>> {
    // SAFETY: reading the session's inputs has no preconditions
    unsafe { session.availableInputs() }.map(|inputs| inputs.to_vec()).unwrap_or_default()
}

/// A mic as the lists show it: the phone's own by a name in the app's language, others by the name iOS gives them
fn mic_item(port: &AVAudioSessionPortDescription) -> DeviceItem {
    // SAFETY: reading a port's type, name and ID has no preconditions, and the constant is a valid NSString
    unsafe {
        let label = if Some(&*port.portType()) == AVAudioSessionPortBuiltInMic {
            "@phone-mic".into()
        } else {
            port.portName().to_string().into()
        };
        DeviceItem { id: port.UID().to_string().into(), label }
    }
}

/// The routes the call can take now, as [`Route::bit`]s. iOS lists no outputs ahead of time, so Bluetooth and
/// headsets are found by their mics, or by playing the call already.
fn available(session: &AVAudioSession, mtm: MainThreadMarker) -> u8 {
    let mut bits = Route::Speaker.bit();
    if UIDevice::currentDevice(mtm).userInterfaceIdiom() == UIUserInterfaceIdiom::Phone {
        bits |= Route::Earpiece.bit();
    }
    for input in inputs(session) {
        // SAFETY: reading a port's type has no preconditions
        bits |= route_of(&*unsafe { input.portType() }).map(Route::bit).unwrap_or(0);
    }
    bits | current(session).map(Route::bit).unwrap_or(0)
}

/// Where the call plays now
fn current(session: &AVAudioSession) -> Option<Route> {
    // SAFETY: reading the session's route and a port's type has no preconditions
    unsafe { session.currentRoute().outputs().firstObject().and_then(|port| route_of(&port.portType())) }
}

/// The route a port, input or output, belongs to
fn route_of(port: &AVAudioSessionPort) -> Option<Route> {
    let is = |name: Option<&AVAudioSessionPort>| name == Some(port);
    // SAFETY: the port names are valid NSString constants
    unsafe {
        if is(AVAudioSessionPortBuiltInSpeaker) {
            Some(Route::Speaker)
        } else if is(AVAudioSessionPortBuiltInReceiver) {
            Some(Route::Earpiece)
        } else if is(AVAudioSessionPortBluetoothHFP)
            || is(AVAudioSessionPortBluetoothA2DP)
            || is(AVAudioSessionPortBluetoothLE)
        {
            Some(Route::Bluetooth)
        } else if is(AVAudioSessionPortHeadsetMic) || is(AVAudioSessionPortHeadphones) || is(AVAudioSessionPortUSBAudio)
        {
            Some(Route::Headset)
        } else {
            None
        }
    }
}

/// Plays the call on `route`: its mic picks the output, and the speaker is an override on top of the built-in mic.
/// Clearing the override sends the sound to the category's default, Bluetooth while it's connected, so it's cleared
/// before the mic is picked, never after.
fn route_to(route: Route) {
    // SAFETY: the shared session lives for the app's whole life and may be used from any thread
    let session = unsafe { AVAudioSession::sharedInstance() };
    let inputs = inputs(&session);
    let input = inputs.iter().find(|port| {
        // SAFETY: reading a port's type has no preconditions, and the constant is a valid NSString
        let kind = unsafe { port.portType() };
        match route {
            // SAFETY: as above
            Route::Speaker | Route::Earpiece => unsafe { Some(&*kind) == AVAudioSessionPortBuiltInMic },
            _ => route_of(&kind) == Some(route),
        }
    });
    let output =
        if route == Route::Speaker { AVAudioSessionPortOverride::Speaker } else { AVAudioSessionPortOverride::None };
    // SAFETY: the session is in PlayAndRecord, where both calls are valid; a port from `availableInputs` or none
    // (headphones without a mic play on their own once nothing overrides them)
    let done = unsafe {
        if route == Route::Speaker {
            session
                .setPreferredInput_error(input.map(|port| &**port))
                .and_then(|()| session.overrideOutputAudioPort_error(output))
        } else {
            session
                .overrideOutputAudioPort_error(output)
                .and_then(|()| session.setPreferredInput_error(input.map(|port| &**port)))
        }
    };
    match done {
        Ok(()) => tracing::info!("call sound to the {}", route.id()),
        Err(e) => tracing::warn!("call sound to the {}: {}", route.id(), e.localizedDescription()),
    }
}
