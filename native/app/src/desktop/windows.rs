//! Windows: friends' calls as toasts.
//!
//! An app without an MSIX package shows toasts under an AppUserModelID registered in the user's registry with a
//! display name. A toast's buttons reach us through its `Activated` event while the app runs, which it does in the
//! tray; no COM activator is registered, so a toast left after the app quits only opens the Action Center.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::core::{HSTRING, IInspectable, Interface, Ref, w};

use super::{Action, Text};

const AUMID: &str = "Danendz.CallDanya";
const RING_PREFIX: &str = "ring-";

struct Toasts {
    notifier: ToastNotifier,
    /// Shown toasts by tag, to take a call's away once it stops ringing
    shown: HashMap<String, ToastNotification>,
}

thread_local! {
    static TOASTS: RefCell<Option<Toasts>> = const { RefCell::new(None) };
}

pub(super) fn start(_text: &Text) {
    if let Err(e) = register() {
        tracing::info!("toast app id not registered: {e}");
    }
    match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) {
        Ok(notifier) => TOASTS.with_borrow_mut(|t| *t = Some(Toasts { notifier, shown: HashMap::new() })),
        Err(e) => tracing::info!("no toasts: {e}"),
    }
}

/// The name toasts show as coming from, under HKCU\Software\Classes\AppUserModelId.
fn register() -> windows::core::Result<()> {
    let name: Vec<u16> = "Call Danya".encode_utf16().chain([0]).collect();
    let key = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{AUMID}"));
    let bytes = u32::try_from(name.len() * 2).unwrap_or(0);
    // SAFETY: `name` is a NUL-terminated UTF-16 string of `bytes` bytes, alive for the call
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, w!("DisplayName"), REG_SZ.0, Some(name.as_ptr().cast()), bytes) }
        .ok()
}

/// A friend's call, with Accept and Decline, kept on screen while it rings. Silent: the app plays the ring tone.
pub(super) fn incoming(text: &Text, call_id: &str, from: &str) {
    let xml = format!(
        r#"<toast scenario="incomingCall"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/><actions><action content="{}" arguments="accept" activationType="foreground"/><action content="{}" arguments="decline" activationType="foreground"/></actions></toast>"#,
        escape(from),
        escape(&text.calling_you),
        escape(&text.accept),
        escape(&text.decline),
    );
    show(&xml, &format!("{RING_PREFIX}{call_id}"));
}

/// Takes the call's toast away once it stops ringing.
pub(super) fn remove(call_id: &str) {
    TOASTS.with_borrow_mut(|t| {
        let Some(t) = t else { return };
        if let Some(toast) = t.shown.remove(&format!("{RING_PREFIX}{call_id}")) {
            let _ = t.notifier.Hide(&toast);
        }
    });
}

pub(super) fn missed(text: &Text, from: &str) {
    let xml = format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        escape(from),
        escape(&text.missed_call),
    );
    // One per friend: a newer missed call replaces the last
    show(&xml, &format!("missed-{from}"));
}

fn show(xml: &str, tag: &str) {
    TOASTS.with_borrow_mut(|t| {
        let Some(t) = t else { return };
        match toast(xml, tag).and_then(|toast| t.notifier.Show(&toast).map(|()| toast)) {
            Ok(toast) => {
                t.shown.insert(tag.to_owned(), toast);
            }
            Err(e) => tracing::info!("toast not shown: {e}"),
        }
    });
}

fn toast(xml: &str, tag: &str) -> windows::core::Result<ToastNotification> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    toast.SetTag(&HSTRING::from(tag))?;
    let call_id = tag.strip_prefix(RING_PREFIX).map(str::to_owned);
    // Called on a thread pool thread
    toast.Activated(&TypedEventHandler::new(move |_: Ref<ToastNotification>, args: Ref<IInspectable>| {
        let arguments = args
            .ok()
            .ok()
            .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|args| args.Arguments().ok())
            .map(|arguments| arguments.to_string())
            .unwrap_or_default();
        super::deliver(match (&call_id, arguments.as_str()) {
            (Some(id), "accept") => Action::Answer(id.clone()),
            (Some(id), "decline") => Action::Decline(id.clone()),
            // A click on the toast itself
            _ => Action::Open,
        });
        Ok(())
    }))?;
    Ok(toast)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Hidden, the window is off the taskbar already.
pub(super) fn set_in_dock(_shown: bool) {}

/// Windows lets the window take focus itself.
pub(super) fn bring_to_front() {}
