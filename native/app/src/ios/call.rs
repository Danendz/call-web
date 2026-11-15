//! The call through CallKit. iOS keeps a call's sound going in the background only for a call CallKit knows of, and
//! then shows it on the lock screen with mute and hang up, holds it for a phone call, and rings for a friend's call
//! on its own screen. [`sync`] tells CallKit what the window is doing, on the UI's clock; [`ProviderDelegate`] hands
//! what's pressed on the system's screens back to the window.
//!
//! Ends and answers made in the app go to CallKit as transactions, as Apple asks, so its handlers find the call already
//! where the app put it and only say done. Without push, a friend's call rings only while the app is still running.
//!
//! Off in the simulator, which has no call screen: iOS ends every CallKit call there at once.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send};
use objc2_avf_audio::AVAudioSession;
use objc2_call_kit::{
    CXAction, CXAnswerCallAction, CXCallController, CXCallEndedReason, CXCallUpdate, CXEndCallAction, CXHandle,
    CXHandleType, CXProvider, CXProviderConfiguration, CXProviderDelegate, CXSetHeldCallAction, CXSetMutedCallAction,
    CXStartCallAction,
};
use objc2_foundation::{NSError, NSNumber, NSSet, NSString, NSUUID};
use slint::{ComponentHandle, Model, SharedString};

use crate::ui::{AppPhase, AppWindow};

/// How long an answered ring may take to become the call before CallKit is told it failed
const ANSWER_WAIT: Duration = Duration::from_secs(20);
/// How recent a ring must be to name the call that follows it
const RING_NAMES: Duration = Duration::from_secs(10);

/// Where the call CallKit knows of is
#[derive(Default)]
enum Call {
    #[default]
    None,
    /// A friend's call, ringing on the system's screen
    Ringing { uuid: Retained<NSUUID>, call_id: SharedString },
    /// A friend's call CallKit wouldn't ring, which only the window shows
    Unrung { call_id: SharedString },
    /// Answered there, until the room opens
    Answered { uuid: Retained<NSUUID>, at: Instant },
    /// The call, with whether CallKit was told it connected and whether its mic is off
    Going { uuid: Retained<NSUUID>, connected: bool, muted: bool },
    /// CallKit turned the call down, as during a phone call: it goes on without it until it ends
    Refused,
}

struct Kit {
    provider: Retained<CXProvider>,
    controller: Retained<CXCallController>,
    // The provider holds its delegate weakly
    _delegate: Retained<ProviderDelegate>,
}

thread_local! {
    static KIT: RefCell<Option<Kit>> = const { RefCell::new(None) };
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    static CALL: RefCell<Call> = const { RefCell::new(Call::None) };
    /// The mic was turned off for a call put ahead of ours, to turn back on when ours is taken off hold
    static MUTED_FOR_HOLD: Cell<bool> = const { Cell::new(false) };
    /// Who the last ring was with, both ways, and when it was seen
    static RANG: RefCell<Option<(SharedString, Instant)>> = const { RefCell::new(None) };
}

define_class!(
    /// Takes what's pressed on CallKit's screens; called on the main thread
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CallDanyaProviderDelegate"]
    struct ProviderDelegate;

    unsafe impl NSObjectProtocol for ProviderDelegate {}

    unsafe impl CXProviderDelegate for ProviderDelegate {
        #[unsafe(method(providerDidReset:))]
        fn did_reset(&self, _provider: &CXProvider) {
            tracing::warn!("CallKit reset its calls");
            if matches!(CALL.replace(Call::None), Call::Going { .. } | Call::Answered { .. }) {
                with_ui(|ui| ui.invoke_hang_up());
            }
        }

        #[unsafe(method(provider:performStartCallAction:))]
        fn start(&self, provider: &CXProvider, action: &CXStartCallAction) {
            // Done first: a call reported before its start action is done counts as a second call with the same id,
            // and CallKit ends both
            // SAFETY: the action is CallKit's own, alive for this call
            unsafe {
                let uuid = action.callUUID();
                action.fulfill();
                provider.reportOutgoingCallWithUUID_startedConnectingAtDate(&uuid, None);
                provider.reportCallWithUUID_updated(&uuid, &update(&action.handle().value()));
            }
        }

        #[unsafe(method(provider:performAnswerCallAction:))]
        fn answer(&self, _provider: &CXProvider, action: &CXAnswerCallAction) {
            // SAFETY: as in `start`
            let uuid = unsafe { action.callUUID() };
            let answered = CALL.with_borrow_mut(|call| match call {
                Call::Ringing { uuid: ringing, .. } if *ringing == uuid => {
                    *call = Call::Answered { uuid, at: Instant::now() };
                    true
                }
                _ => false,
            });
            // The window shows the same ring, the oldest
            if answered {
                with_ui(|ui| ui.invoke_answer(true));
            }
            // SAFETY: as in `start`
            unsafe { action.fulfill() };
        }

        #[unsafe(method(provider:performEndCallAction:))]
        fn end(&self, _provider: &CXProvider, action: &CXEndCallAction) {
            // SAFETY: as in `start`
            let uuid = unsafe { action.callUUID() };
            let was = CALL.with_borrow_mut(|call| {
                if call.uuid().is_some_and(|ours| *ours == uuid) { std::mem::take(call) } else { Call::None }
            });
            match was {
                Call::Ringing { .. } => with_ui(|ui| ui.invoke_answer(false)),
                Call::Answered { .. } | Call::Going { .. } => with_ui(|ui| ui.invoke_hang_up()),
                Call::None | Call::Unrung { .. } | Call::Refused => {}
            }
            // SAFETY: as in `start`
            unsafe { action.fulfill() };
        }

        #[unsafe(method(provider:performSetMutedCallAction:))]
        fn set_muted(&self, _provider: &CXProvider, action: &CXSetMutedCallAction) {
            // SAFETY: as in `start`
            let (uuid, muted) = unsafe { (action.callUUID(), action.isMuted()) };
            let ours = CALL.with_borrow_mut(|call| match call {
                Call::Going { uuid: ours, muted: was, .. } if *ours == uuid => {
                    *was = muted;
                    true
                }
                _ => false,
            });
            if ours {
                with_ui(|ui| {
                    if ui.get_mic_on() == muted {
                        ui.invoke_toggle_mic();
                    }
                });
            }
            // SAFETY: as in `start`
            unsafe { action.fulfill() };
        }

        // A phone call answered with "Hold & Accept": the mic goes off rather than talk over it, as on Android
        #[unsafe(method(provider:performSetHeldCallAction:))]
        fn set_held(&self, _provider: &CXProvider, action: &CXSetHeldCallAction) {
            // SAFETY: as in `start`
            let held = unsafe { action.isOnHold() };
            with_ui(|ui| {
                if held && ui.get_mic_on() && !MUTED_FOR_HOLD.get() {
                    MUTED_FOR_HOLD.set(true);
                    ui.invoke_toggle_mic();
                } else if !held && MUTED_FOR_HOLD.replace(false) && !ui.get_mic_on() {
                    ui.invoke_toggle_mic();
                }
            });
            // SAFETY: as in `start`
            unsafe { action.fulfill() };
        }

        #[unsafe(method(provider:timedOutPerformingAction:))]
        fn timed_out(&self, _provider: &CXProvider, _action: &CXAction) {
            tracing::warn!("a CallKit action timed out");
        }

        #[unsafe(method(provider:didActivateAudioSession:))]
        fn activated(&self, _provider: &CXProvider, _session: &AVAudioSession) {
            tracing::info!("CallKit turned the audio session on");
        }

        #[unsafe(method(provider:didDeactivateAudioSession:))]
        fn deactivated(&self, _provider: &CXProvider, _session: &AVAudioSession) {
            tracing::info!("CallKit turned the audio session off");
        }
    }
);

impl Call {
    fn uuid(&self) -> Option<&Retained<NSUUID>> {
        match self {
            Self::Ringing { uuid, .. } | Self::Answered { uuid, .. } | Self::Going { uuid, .. } => Some(uuid),
            Self::None | Self::Unrung { .. } | Self::Refused => None,
        }
    }
}

/// Makes the provider CallKit talks to. Calls are kept out of the phone's Recents: tapping one there would call back
/// through a contact the app doesn't have.
pub fn attach(ui: &AppWindow) {
    if cfg!(target_abi = "sim") {
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    // SAFETY: plain setters on a configuration this owns; the delegate is ours and lives as long as the provider, in
    // `KIT`; a nil queue means the main queue
    let kit = unsafe {
        let config = CXProviderConfiguration::new();
        config.setMaximumCallGroups(1);
        config.setMaximumCallsPerCallGroup(1);
        config.setIncludesCallsInRecents(false);
        config.setSupportedHandleTypes(&NSSet::from_retained_slice(&[NSNumber::new_isize(CXHandleType::Generic.0)]));
        let provider = CXProvider::initWithConfiguration(CXProvider::alloc(), &config);
        let delegate: Retained<ProviderDelegate> = msg_send![ProviderDelegate::alloc(mtm), init];
        provider.setDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), None);
        Kit { provider, controller: CXCallController::new(), _delegate: delegate }
    };
    KIT.set(Some(kit));
    UI.set(Some(ui.as_weak()));
}

/// Tells CallKit what the window is doing: a friend's call ringing, the call starting, the mic, the call ending.
pub fn sync(ui: &AppWindow) {
    let in_call = matches!(ui.get_phase(), AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall);
    let incoming = ui.get_incoming_call_id();
    let calling = ui.get_calling();
    let ring = if calling.is_empty() { ui.get_incoming_from() } else { calling };
    if !ring.is_empty() {
        RANG.set(Some((ring, Instant::now())));
    }
    KIT.with_borrow(|kit| {
        let Some(kit) = kit else { return };
        CALL.with_borrow_mut(|call| step(kit, ui, call, in_call, &incoming));
    });
}

fn step(kit: &Kit, ui: &AppWindow, call: &mut Call, in_call: bool, incoming: &SharedString) {
    match call {
        Call::None if in_call => {
            let uuid = NSUUID::new();
            let name = RANG
                .take()
                .filter(|(_, at)| at.elapsed() < RING_NAMES)
                .map_or_else(|| ui.get_call_notice().row_data(1).unwrap_or_default(), |(name, _)| name);
            // SAFETY: a new action for a new call
            let action = unsafe {
                let handle =
                    CXHandle::initWithType_value(CXHandle::alloc(), CXHandleType::Generic, &NSString::from_str(&name));
                CXStartCallAction::initWithCallUUID_handle(CXStartCallAction::alloc(), &uuid, &handle)
            };
            request(kit, &action, |_| CALL.set(Call::Refused));
            *call = Call::Going { uuid, connected: false, muted: false };
        }
        Call::None if !incoming.is_empty() => {
            let uuid = NSUUID::new();
            let call_id = incoming.clone();
            let refused = RcBlock::new(move |error: *mut NSError| {
                // SAFETY: CallKit passes a live error or nil
                if let Some(error) = unsafe { error.as_ref() } {
                    tracing::warn!("CallKit didn't ring: {}", error.localizedDescription());
                    CALL.set(Call::Unrung { call_id: call_id.clone() });
                }
            });
            // SAFETY: the update is new and the block lives until CallKit copies it
            unsafe {
                kit.provider.reportNewIncomingCallWithUUID_update_completion(
                    &uuid,
                    &update(&NSString::from_str(&ui.get_incoming_from())),
                    &refused,
                );
            }
            *call = Call::Ringing { uuid, call_id: incoming.clone() };
        }
        // Cancelled by the caller
        Call::Ringing { call_id, .. } if call_id != incoming => end(kit, call, CXCallEndedReason::RemoteEnded),
        Call::Unrung { call_id } if in_call || call_id != incoming => *call = Call::None,
        Call::Answered { uuid, .. } if in_call => {
            *call = Call::Going { uuid: uuid.clone(), connected: true, muted: false };
        }
        Call::Answered { at, .. } if at.elapsed() > ANSWER_WAIT => end(kit, call, CXCallEndedReason::Failed),
        Call::Going { uuid, connected, muted } if in_call => {
            if !*connected && ui.get_phase() == AppPhase::InCall {
                *connected = true;
                // SAFETY: the call CallKit knows of
                unsafe { kit.provider.reportOutgoingCallWithUUID_connectedAtDate(uuid, None) };
            }
            let mic_off = !ui.get_mic_on();
            if *muted != mic_off {
                *muted = mic_off;
                // SAFETY: a new action on the call CallKit knows of
                let action = unsafe {
                    CXSetMutedCallAction::initWithCallUUID_muted(CXSetMutedCallAction::alloc(), uuid, mic_off)
                };
                request(kit, &action, |_| {});
            }
        }
        Call::Going { uuid, .. } => {
            // Hung up here or by the other person: either way the app ended it
            // SAFETY: a new action on the call CallKit knows of
            let action = unsafe { CXEndCallAction::initWithCallUUID(CXEndCallAction::alloc(), uuid) };
            request(kit, &action, |_| {});
            *call = Call::None;
            MUTED_FOR_HOLD.set(false);
        }
        Call::Refused if !in_call => *call = Call::None,
        _ => {}
    }
}

/// The ring shown in the window was answered there (`accept`) or turned down: CallKit, ringing the same call, answers
/// or ends it too. Its handlers then find it already moved on and leave the window be.
pub fn answered(accept: bool) {
    KIT.with_borrow(|kit| {
        let Some(kit) = kit else { return };
        CALL.with_borrow_mut(|call| {
            let Call::Ringing { uuid, .. } = call else { return };
            let uuid = uuid.clone();
            // SAFETY: new actions on the call CallKit rings
            let action: Retained<CXAction> = unsafe {
                if accept {
                    Retained::into_super(CXAnswerCallAction::initWithCallUUID(CXAnswerCallAction::alloc(), &uuid))
                        .into_super()
                } else {
                    Retained::into_super(CXEndCallAction::initWithCallUUID(CXEndCallAction::alloc(), &uuid))
                        .into_super()
                }
            };
            *call = if accept { Call::Answered { uuid, at: Instant::now() } } else { Call::None };
            request(kit, &action, |_| {});
        });
    });
}

/// Tells CallKit a call it knows of ended without the app asking.
fn end(kit: &Kit, call: &mut Call, reason: CXCallEndedReason) {
    if let Some(uuid) = call.uuid() {
        // SAFETY: the call CallKit knows of
        unsafe { kit.provider.reportCallWithUUID_endedAtDate_reason(uuid, None, reason) };
    }
    *call = Call::None;
}

/// Asks CallKit for an action; `refused` runs on the main thread if it turns it down.
fn request(kit: &Kit, action: &CXAction, refused: impl Fn(&NSError) + 'static) {
    let done = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: CallKit passes a live error or nil
        if let Some(error) = unsafe { error.as_ref() } {
            tracing::warn!("CallKit refused an action: {}", error.localizedDescription());
            refused(error);
        }
    });
    // SAFETY: the action is new and the block lives until CallKit copies it; the controller's completions run on
    // the main queue
    unsafe { kit.controller.requestTransactionWithAction_completion(action, &done) };
}

/// The call as CallKit shows it: who with, audio only, so CallKit leaves the route to `ios::audio`
fn update(name: &NSString) -> Retained<CXCallUpdate> {
    // SAFETY: plain setters on an update this owns
    unsafe {
        let update = CXCallUpdate::new();
        update.setRemoteHandle(Some(&CXHandle::initWithType_value(CXHandle::alloc(), CXHandleType::Generic, name)));
        update.setLocalizedCallerName(Some(name));
        update.setHasVideo(false);
        update.setSupportsHolding(true);
        update.setSupportsGrouping(false);
        update.setSupportsUngrouping(false);
        update.setSupportsDTMF(false);
        update
    }
}

fn with_ui(f: impl FnOnce(&AppWindow)) {
    if let Some(ui) = UI.with_borrow(|ui| ui.as_ref().and_then(slint::Weak::upgrade)) {
        f(&ui);
    }
}
