//! The tray (Windows) or menu bar (macOS) icon: a click opens the window, and its menu quits.
//!
//! The menu holds only the OS's own Quit, which needs no event handler: muda takes one handler for the whole
//! process, and Slint installs its own the first time a text field's menu opens, so ours would be dropped or
//! would drop Slint's.

use tray_icon::menu::{Menu, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::Text;

pub(super) struct Tray(#[allow(dead_code, reason = "held to keep the icon shown")] TrayIcon);

impl Tray {
    /// None (logged) when the OS has no tray to put it in.
    pub(super) fn new(text: &Text, image: &slint::Image) -> Option<Self> {
        TrayIconEvent::set_event_handler(Some(|event| {
            let open = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if open {
                let _ = slint::invoke_from_event_loop(super::show);
            }
        }));
        let menu = Menu::new();
        if let Err(e) = menu.append(&PredefinedMenuItem::quit(Some(&text.quit))) {
            tracing::warn!("tray menu: {e}");
        }
        let built = TrayIconBuilder::new()
            .with_icon(icon(image)?)
            .with_tooltip("Call Danya")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build();
        match built {
            Ok(tray) => Some(Self(tray)),
            Err(e) => {
                tracing::warn!("no tray icon: {e}");
                None
            }
        }
    }
}

fn icon(image: &slint::Image) -> Option<Icon> {
    let Some(image) = image.to_rgba8() else {
        tracing::warn!("tray icon: the app icon has no pixels to read");
        return None;
    };
    let (width, height) = (image.width(), image.height());
    Icon::from_rgba(image.as_bytes().to_vec(), width, height).inspect_err(|e| tracing::warn!("tray icon: {e}")).ok()
}
