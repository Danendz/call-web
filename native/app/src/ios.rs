//! The UIScene lifecycle, which iOS 27 requires: an app without it is stopped at launch. winit 0.30 makes its
//! `UIWindow` the old way, outside any scene, where it isn't shown, so `Info.plist` names [`SceneDelegate`] and
//! the window is moved into the scene once both exist, whichever comes second.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, MainThreadOnly, Message, define_class};
use objc2_ui_kit::{
    UIResponder, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIView, UIWindow, UIWindowScene,
    UIWindowSceneDelegate,
};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    static SCENE: RefCell<Option<Retained<UIWindowScene>>> = const { RefCell::new(None) };
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
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

/// Registers [`SceneDelegate`] with the Objective-C runtime; UIKit looks it up by name when the scene connects, so
/// before the event loop starts.
pub fn register() {
    SceneDelegate::class();
}

/// Hands the window Slint made to the scene. Call it once the event loop runs, when the window exists.
pub fn attach_window(ui: &crate::ui::AppWindow) {
    let view = ui.window().with_winit_window(|window| match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::UiKit(handle)) => Some(handle.ui_view),
        _ => None,
    });
    let Some(Some(view)) = view else {
        tracing::warn!("no UIView behind the window");
        return;
    };
    // SAFETY: winit's UiKit handle points at its live UIView, which its window keeps alive while we're on the main
    // thread, where this runs from Slint's event loop
    let view = unsafe { view.cast::<UIView>().as_ref() };
    let Some(window) = view.window() else {
        tracing::warn!("the view isn't in a UIWindow");
        return;
    };
    WINDOW.with(|w| *w.borrow_mut() = Some(window));
    attach();
}

fn attach() {
    let Some(scene) = SCENE.with(|s| s.borrow().clone()) else { return };
    let Some(window) = WINDOW.with(|w| w.borrow().clone()) else { return };
    window.setWindowScene(Some(&scene));
    window.makeKeyAndVisible();
}
