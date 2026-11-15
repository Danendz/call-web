//! The UIScene lifecycle, which iOS 27 requires: an app without it is stopped at launch. winit 0.30 makes its
//! `UIWindow` the old way, outside any scene, where it isn't shown, so `Info.plist` names [`SceneDelegate`] and
//! the window is moved into the scene once both exist, whichever comes second.
//!
//! Also tells Slint when the safe area changes, which it otherwise only reads on a resize: turning the phone resizes
//! the window before UIKit moves the safe area, so the old insets stayed.
//!
//! Also gives Slint a Chinese font: iOS's only one, PingFang, has outlines Slint's font library rejects and Skia
//! can't draw, so every Chinese character was a box. The bundle carries Noto Sans SC (`ios/build-rust.sh`).
//!
//! Also opens a text box's menu on a long press, which Slint does only on Android ([`touch`]), as iOS's own edit menu
//! rather than Slint's ([`text_menus`]).
//!
//! The call's audio route is in [`audio`], the call through CallKit in [`call`], and the update check in [`update`].

pub mod audio;
pub mod call;
pub mod update;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_foundation::CGPoint;
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{
    UIAction, UIEdgeInsets, UIEditMenuConfiguration, UIEditMenuInteraction, UIEditMenuInteractionDelegate, UIMenu,
    UIMenuElement, UIResponder, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIView, UIWindow,
    UIWindowScene, UIWindowSceneDelegate,
};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::dpi::PhysicalPosition;
use slint::winit_030::winit::event::{Touch, TouchPhase};
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::ui::{AppWindow, TextAction, TextMenus};

/// How long a finger rests before it's a long press, as UIKit's own
const LONG_PRESS: Duration = Duration::from_millis(500);
/// How far it may move meanwhile, in points
const LONG_PRESS_SLOP: f64 = 10.0;

/// A finger down that may become a long press; dropping it stops the wait
struct Press {
    id: u64,
    at: PhysicalPosition<f64>,
    _timer: slint::Timer,
}

thread_local! {
    static SCENE: RefCell<Option<Retained<UIWindowScene>>> = const { RefCell::new(None) };
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
    static SAFE_AREA: Cell<Option<UIEdgeInsets>> = const { Cell::new(None) };
    static SAFE_AREA_WATCH: slint::Timer = slint::Timer::default();
    static ATTACH_WAIT: slint::Timer = slint::Timer::default();
    static PRESS: RefCell<Option<Press>> = const { RefCell::new(None) };
    /// The window view's edit menu and its delegate, which the interaction only holds weakly
    static EDIT_MENU: RefCell<Option<(Retained<UIEditMenuInteraction>, Retained<EditMenuDelegate>)>> =
        const { RefCell::new(None) };
    /// The items of the menu being opened, which the delegate hands UIKit
    static EDIT_MENU_ITEMS: RefCell<Option<Retained<UIMenu>>> = const { RefCell::new(None) };
}

define_class!(
    /// Named in `Info.plist`'s `UIApplicationSceneManifest`
    #[unsafe(super(UIResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "CallDanyaSceneDelegate"]
    struct SceneDelegate;

    unsafe impl NSObjectProtocol for SceneDelegate {}

    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn will_connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
            let Some(scene) = scene.downcast_ref::<UIWindowScene>() else { return };
            SCENE.with(|s| *s.borrow_mut() = Some(scene.retain()));
            attach();
        }
    }

    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

define_class!(
    /// Gives the edit menu the items [`text_menus`] put in `EDIT_MENU_ITEMS`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CallDanyaEditMenuDelegate"]
    struct EditMenuDelegate;

    unsafe impl NSObjectProtocol for EditMenuDelegate {}

    unsafe impl UIEditMenuInteractionDelegate for EditMenuDelegate {
        #[unsafe(method_id(editMenuInteraction:menuForConfiguration:suggestedActions:))]
        fn menu(
            &self,
            _interaction: &UIEditMenuInteraction,
            _configuration: &UIEditMenuConfiguration,
            _suggested: &NSArray<UIMenuElement>,
        ) -> Option<Retained<UIMenu>> {
            EDIT_MENU_ITEMS.with_borrow(Clone::clone)
        }
    }
);

/// Registers [`SceneDelegate`] with the Objective-C runtime; UIKit looks it up by name when the scene connects, so
/// before the event loop starts.
pub fn register() {
    SceneDelegate::class();
}

/// Calls [`attach_window`] once Slint has made the window, which is some time after the event loop starts
pub fn attach_window_when_made(ui: &AppWindow) {
    let weak = ui.as_weak();
    let tries = Cell::new(0);
    ATTACH_WAIT.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
            let Some(ui) = weak.upgrade() else { return };
            tries.set(tries.get() + 1);
            if attach_window(&ui) || tries.get() >= 100 {
                if tries.get() >= 100 {
                    tracing::warn!("no UIView behind the window");
                }
                ATTACH_WAIT.with(slint::Timer::stop);
            }
        });
    });
}

/// Hands the window Slint made to the scene; false while there's no window yet
fn attach_window(ui: &AppWindow) -> bool {
    let view = ui.window().with_winit_window(|window| match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::UiKit(handle)) => Some(handle.ui_view),
        _ => None,
    });
    let Some(Some(view)) = view else { return false };
    // SAFETY: winit's UiKit handle points at its live UIView, which its window keeps alive while we're on the main
    // thread, where this runs from Slint's event loop
    let view = unsafe { view.cast::<UIView>().as_ref() };
    let Some(window) = view.window() else {
        tracing::warn!("the view isn't in a UIWindow");
        return true;
    };
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    attach();
    let mtm = view.mtm();
    // SAFETY: NSObject's plain init on a class with no state of its own
    let delegate: Retained<EditMenuDelegate> = unsafe { msg_send![EditMenuDelegate::alloc(mtm), init] };
    let interaction = UIEditMenuInteraction::initWithDelegate(
        UIEditMenuInteraction::alloc(mtm),
        Some(ProtocolObject::from_ref(&*delegate)),
    );
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
    EDIT_MENU.set(Some((interaction, delegate)));
    let view = view.retain();
    SAFE_AREA_WATCH.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            let insets = window.safeAreaInsets();
            if SAFE_AREA.replace(Some(insets)).is_some_and(|old| old != insets) {
                // winit reports a resize from the view's layout, and Slint reads the insets then
                view.setNeedsLayout();
            }
        });
    });
    true
}

fn attach() {
    let Some(scene) = SCENE.with(|s| s.borrow().clone()) else { return };
    let Some(window) = WINDOW.with(|w| w.borrow().clone()) else { return };
    window.setWindowScene(Some(&scene));
    window.makeKeyAndVisible();
}

/// The system UI font, which stays the font for everything but Chinese
const UI_FONT: &str = "/System/Library/Fonts/Core/SFUI.ttf";

/// Sets `SLINT_DEFAULT_FONT` and `SLINT_FONT_PATH` so Chinese falls back to the bundled Noto Sans SC. Must run at the
/// start of `main`, before any thread starts.
pub fn fonts() {
    let Some(cjk) = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("NotoSansSC.ttf"))) else {
        return;
    };
    if !cjk.is_file() {
        tracing::warn!("no Chinese font in the bundle");
        return;
    }
    // SAFETY: nothing else reads or writes the environment yet: `main` calls this before any thread starts
    unsafe {
        // The font path replaces Slint's sans-serif, so the UI font goes first and Chinese falls back to the next
        if std::path::Path::new(UI_FONT).is_file() {
            std::env::set_var("SLINT_DEFAULT_FONT", UI_FONT);
        }
        std::env::set_var("SLINT_FONT_PATH", cjk);
    }
}

/// Watches the window's touches for a finger held still, and tells the text boxes where (`TextMenus`): the one under
/// it opens its menu, as a right-click does on a computer. Slint still gets every touch.
pub fn touch(ui: &slint::Weak<AppWindow>, scale: f32, touch: &Touch) {
    match touch.phase {
        TouchPhase::Started => {
            let timer = slint::Timer::default();
            let ui = ui.clone();
            timer.start(slint::TimerMode::SingleShot, LONG_PRESS, move || {
                let Some(press) = PRESS.take() else { return };
                let Some(ui) = ui.upgrade() else { return };
                let menus = ui.global::<TextMenus>();
                let at = press.at.to_logical::<f32>(f64::from(scale));
                menus.set_at(slint::LogicalPosition::new(at.x, at.y));
                menus.set_presses(menus.get_presses().wrapping_add(1));
            });
            PRESS.set(Some(Press { id: touch.id, at: touch.location, _timer: timer }));
        }
        TouchPhase::Moved => {
            let moved = PRESS.with_borrow(|p| {
                p.as_ref().is_some_and(|p| {
                    let (dx, dy) = (touch.location.x - p.at.x, touch.location.y - p.at.y);
                    p.id != touch.id || dx.hypot(dy) > LONG_PRESS_SLOP * f64::from(scale)
                })
            });
            if moved {
                PRESS.take();
            }
        }
        TouchPhase::Ended | TouchPhase::Cancelled => {
            PRESS.take();
        }
    }
}

/// Answers a text box's long press (`TextMenus.open`) with iOS's edit menu at the press, and hands the item picked back
/// to the box (`TextMenus.picked`). Slint's own menu would be drawn by Slint, unlike anything else on the phone.
pub fn text_menus(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<TextMenus>().on_open(move |read_only, can_copy| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(mtm) = MainThreadMarker::new() else { return };
        let menus = ui.global::<TextMenus>();
        let mut items = Vec::new();
        if !read_only && can_copy {
            items.push((menus.get_cut_title(), TextAction::Cut));
        }
        if can_copy {
            items.push((menus.get_copy_title(), TextAction::Copy));
        }
        if !read_only {
            items.push((menus.get_paste_title(), TextAction::Paste));
        }
        items.push((menus.get_select_all_title(), TextAction::SelectAll));
        let actions: Vec<Retained<UIMenuElement>> = items
            .into_iter()
            .map(|(title, action)| {
                let weak = weak.clone();
                let picked: RcBlock<dyn Fn(NonNull<UIAction>)> = RcBlock::new(move |_| {
                    let Some(ui) = weak.upgrade() else { return };
                    let menus = ui.global::<TextMenus>();
                    menus.set_picked(action);
                    menus.set_picks(menus.get_picks().wrapping_add(1));
                });
                // SAFETY: the handler is a live block, which UIKit copies and keeps
                let item = unsafe {
                    UIAction::actionWithTitle_image_identifier_handler(
                        &NSString::from_str(&title),
                        None,
                        None,
                        RcBlock::as_ptr(&picked),
                        mtm,
                    )
                };
                Retained::into_super(item)
            })
            .collect();
        EDIT_MENU_ITEMS.set(Some(UIMenu::menuWithChildren(&NSArray::from_retained_slice(&actions), mtm)));

        let at = menus.get_at();
        EDIT_MENU.with_borrow(|menu| {
            let Some((interaction, _)) = menu else { return };
            // Slint's logical pixels are the view's points
            let at = CGPoint::new(f64::from(at.x), f64::from(at.y));
            // SAFETY: no identifier, which is optional
            let configuration =
                unsafe { UIEditMenuConfiguration::configurationWithIdentifier_sourcePoint(None, at, mtm) };
            interaction.presentEditMenuWithConfiguration(&configuration);
        });
    });
}
