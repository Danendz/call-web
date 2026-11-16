//! macOS: friends' calls through the notification center, and the Dock icon only while the window shows.
//!
//! The notification center needs the app bundle: a bare binary from `cargo run` has no bundle id, and asking for
//! the center there throws, so it goes without notifications.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSArray, NSBundle, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::{Action, Text};

const RING_CATEGORY: &str = "ring";
const RING_PREFIX: &str = "ring-";

thread_local! {
    // The center only keeps a weak reference to its delegate
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

define_class!(
    /// Takes what's clicked on our notifications; called on the main thread
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CallDanyaNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Shown even while the app is active: it's only posted when the window isn't in front
            completion.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier().to_string();
            let picked = match id.strip_prefix(RING_PREFIX) {
                Some(call_id) if action == "accept" => Action::Answer(call_id.to_owned()),
                Some(call_id) if action == "decline" => Action::Decline(call_id.to_owned()),
                // A click on the notification itself
                _ => Action::Open,
            };
            super::deliver(picked);
            completion.call(());
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: NSObject's plain init
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

fn center() -> Option<Retained<UNUserNotificationCenter>> {
    NSBundle::mainBundle().bundleIdentifier()?;
    Some(UNUserNotificationCenter::currentNotificationCenter())
}

/// Sets up the ring's Accept and Decline buttons and asks for permission (once; macOS remembers the answer).
pub(super) fn start(text: &Text) {
    let (Some(center), Some(mtm)) = (center(), MainThreadMarker::new()) else {
        tracing::info!("no notifications: not running from the app bundle");
        return;
    };
    let delegate = Delegate::new(mtm);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with_borrow_mut(|d| *d = Some(delegate));

    let accept = UNNotificationAction::actionWithIdentifier_title_options(
        &NSString::from_str("accept"),
        &NSString::from_str(&text.accept),
        UNNotificationActionOptions::Foreground,
    );
    let decline = UNNotificationAction::actionWithIdentifier_title_options(
        &NSString::from_str("decline"),
        &NSString::from_str(&text.decline),
        UNNotificationActionOptions::Destructive,
    );
    let ring = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(RING_CATEGORY),
        &NSArray::from_retained_slice(&[accept, decline]),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[ring]));
    let done = block2::RcBlock::new(|granted: objc2::runtime::Bool, _error: *mut objc2_foundation::NSError| {
        if !granted.as_bool() {
            tracing::info!("notifications not allowed");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
}

/// A friend's call, with Accept and Decline. Silent: the app plays the ring tone itself.
pub(super) fn incoming(text: &Text, call_id: &str, from: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(from));
    content.setBody(&NSString::from_str(&text.calling_you));
    content.setCategoryIdentifier(&NSString::from_str(RING_CATEGORY));
    post(&format!("{RING_PREFIX}{call_id}"), &content);
}

/// Takes the call's notification away once it stops ringing.
pub(super) fn remove(call_id: &str) {
    let Some(center) = center() else { return };
    let ids = NSArray::from_retained_slice(&[NSString::from_str(&format!("{RING_PREFIX}{call_id}"))]);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
}

pub(super) fn missed(text: &Text, from: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(from));
    content.setBody(&NSString::from_str(&text.missed_call));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    // One per friend: a newer missed call replaces the last
    post(&format!("missed-{from}"), &content);
}

fn post(id: &str, content: &UNMutableNotificationContent) {
    let Some(center) = center() else { return };
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), content, None);
    let done = block2::RcBlock::new(|error: *mut objc2_foundation::NSError| {
        if !error.is_null() {
            tracing::info!("notification not posted");
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

/// In the Dock and the app switcher while the window shows; only in the menu bar while hidden.
pub(super) fn set_in_dock(shown: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let policy = if shown { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

/// Brings the app in front of others, as a click on the menu bar icon or a notification means to.
pub(super) fn bring_to_front() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    NSApplication::sharedApplication(mtm).activate();
}
