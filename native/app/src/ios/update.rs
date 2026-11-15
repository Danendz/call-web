//! Update checks. A sideloaded app can't install itself, so this only reads the AltStore source CI publishes
//! (`/updates/altstore.json`, `packaging/ios/release.sh`) and, when it lists a newer version, the start screen's
//! "Update to …" button hands the `.ipa`'s link to AltStore, or SideStore when AltStore isn't there, which installs
//! it. A debug build talks to the dev server, which has no updates.

use std::time::Duration;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::runtime::Bool;
use objc2_foundation::{NSDictionary, NSString, NSURL};
use objc2_ui_kit::UIApplication;
use slint::ComponentHandle;
use url::Url;

use crate::ui::AppWindow;

const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);
const STORES: [&str; 2] = ["altstore", "sidestore"];

pub fn start(ui: &AppWindow, server: &Url) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(source) = server.join("updates/altstore.json") else { return };
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        loop {
            match newer(&source) {
                Ok(Some((version, ipa))) => {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.on_update_now(move || open_in_store(&ipa, 0));
                        ui.set_update_version(version.into());
                    });
                    return;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("update check failed: {e}"),
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// The newest version and its `.ipa`'s link, when it's newer than this one.
fn newer(source: &Url) -> Result<Option<(String, String)>, String> {
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().new_agent();
    let body = agent.get(source.as_str()).call().map_err(|e| e.to_string())?.body_mut().read_to_string();
    let json: serde_json::Value = serde_json::from_str(&body.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let field = |name: &str| json.pointer(&format!("/apps/0/versions/0/{name}")).and_then(|v| v.as_str());
    let (Some(version), Some(ipa)) = (field("version"), field("downloadURL")) else {
        return Err("altstore.json has no version".into());
    };
    Ok((parse(version) > parse(env!("CARGO_PKG_VERSION"))).then(|| (version.to_owned(), ipa.to_owned())))
}

fn parse(version: &str) -> Vec<u32> {
    version.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// Opens `<store>://install?url=<ipa>`, trying the next store when iOS has nothing for that scheme.
fn open_in_store(ipa: &str, store: usize) {
    let Some(scheme) = STORES.get(store) else {
        tracing::warn!("neither AltStore nor SideStore is installed");
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else { return };
    let query: String = url::form_urlencoded::byte_serialize(ipa.as_bytes()).collect();
    let Some(link) = NSURL::URLWithString(&NSString::from_str(&format!("{scheme}://install?url={query}"))) else {
        return;
    };
    let ipa = ipa.to_owned();
    let done = RcBlock::new(move |opened: Bool| {
        if !opened.as_bool() {
            open_in_store(&ipa, store + 1);
        }
    });
    // SAFETY: an empty options dictionary is of the type it asks for; UIKit calls the completion on the main thread
    unsafe {
        UIApplication::sharedApplication(mtm).openURL_options_completionHandler(
            &link,
            &NSDictionary::new(),
            Some(&done),
        );
    }
}
