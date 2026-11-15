//! Picks Slint's renderer and watches it. On desktop also tweaks the native window: on macOS the content runs under a
//! transparent title bar, on Windows the frame is dark, and video is told when the window can't be seen. The desktop
//! part uses Slint's unstable winit hooks, which is why slint is pinned exactly.
//!
//! Skia draws over wgpu (Metal, D3D12 or Vulkan) unless Settings says OpenGL. wgpu that can't start, or that
//! never drew a frame at the last launch, falls back to OpenGL and stays there, so a bad driver costs at most one
//! relaunch.
//!
//! On Android the window's surface is made again each time the app comes back from the background, and a driver can
//! fail there too, so Vulkan is trusted only once it drew on a surface made again: it first drew ([`WgpuCheck::Drew`]),
//! then the app came back to the front after its surface went away (`Trying` again), then it drew again (`Works`).
//! A surface Vulkan can't make falls back to OpenGL in place, without a relaunch: our patched backend does that
//! (vendor/README.md), and [`watch_renderer`] saves it. The choice in Settings applies at once.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::window::WindowAttributes;

use crate::settings::{Renderer, Settings, WgpuCheck};

#[cfg(not(target_os = "android"))]
pub fn select_backend() -> Result<(), slint::PlatformError> {
    // iOS has no OpenGL to fall back to
    if cfg!(target_os = "ios") {
        return selector().renderer_name("skia-wgpu".into()).require_wgpu_30(wgpu_configuration()).select();
    }
    let mut settings = Settings::load();
    if settings.renderer == Renderer::Modern && settings.wgpu == WgpuCheck::Trying {
        fall_back(&mut settings, "the last launch on wgpu never drew a frame");
    }
    if settings.renderer == Renderer::Modern {
        let wgpu = selector().renderer_name("skia-wgpu".into()).require_wgpu_30(wgpu_configuration()).select();
        match wgpu {
            Ok(()) => {
                if settings.wgpu == WgpuCheck::Untested {
                    settings.wgpu = WgpuCheck::Trying;
                    settings.save();
                }
                return Ok(());
            }
            // A selector that failed set nothing, so another one can still pick
            Err(e) => fall_back(&mut settings, &e.to_string()),
        }
    }
    selector().renderer_name("skia-opengl".into()).select()
}

/// After `slint::android::init`, before the window exists: asks the backend for the renderer Settings name.
#[cfg(target_os = "android")]
pub fn select_backend() -> Result<(), slint::PlatformError> {
    let mut settings = Settings::load();
    if settings.renderer == Renderer::Modern && settings.wgpu == WgpuCheck::Trying {
        fall_back(&mut settings, "the last launch on wgpu stopped before it drew a frame");
    }
    if settings.renderer == Renderer::Modern && settings.wgpu == WgpuCheck::Untested {
        settings.wgpu = WgpuCheck::Trying;
        settings.save();
    }
    request(settings.renderer)
}

#[cfg(target_os = "android")]
fn request(renderer: Renderer) -> Result<(), slint::PlatformError> {
    use i_slint_core::graphics::{RequestedGraphicsAPI, RequestedOpenGLVersion};
    let api = match renderer {
        Renderer::Modern => RequestedGraphicsAPI::WGPU30(wgpu_configuration()),
        Renderer::OpenGl => RequestedGraphicsAPI::OpenGL(RequestedOpenGLVersion::OpenGLES(Some((3, 0)))),
    };
    i_slint_backend_android_activity::set_requested_graphics_api(Some(api))
}

/// Settings' "Graphics" on Android: saves the choice and makes the window's surface again with it. Picked by hand,
/// wgpu gets another try, checked as at a launch.
#[cfg(target_os = "android")]
pub fn switch(settings: &mut Settings, renderer: Renderer) {
    settings.renderer = renderer;
    settings.wgpu = if renderer == Renderer::Modern { WgpuCheck::Trying } else { WgpuCheck::Untested };
    settings.save();
    // Not inside the callback that asked: the old surface's images may still be in use until it returns
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Err(e) = request(renderer) {
            tracing::warn!("couldn't switch the renderer: {e}");
        }
    });
}

/// In a debug build, `CALL_FAKE_NO_WGPU=1` offers wgpu no backends, so the fallback to OpenGL can be tested on a
/// machine where wgpu works (on Android, `adb shell setprop debug.call_danya.fake_no_wgpu 1`).
fn wgpu_configuration() -> slint::wgpu_30::WGPUConfiguration {
    use slint::wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu};
    if cfg!(debug_assertions) && std::env::var_os("CALL_FAKE_NO_WGPU").is_some() {
        let mut settings = WGPUSettings::default();
        settings.backends = wgpu::Backends::empty();
        return WGPUConfiguration::Automatic(settings);
    }
    WGPUConfiguration::default()
}

#[cfg(not(target_os = "android"))]
fn selector() -> slint::BackendSelector {
    slint::BackendSelector::new().backend_name("winit".into()).with_winit_window_attributes_hook(native_frame)
}

fn fall_back(settings: &mut Settings, why: &str) {
    tracing::warn!("drawing with OpenGL instead of wgpu: {why}");
    settings.renderer = Renderer::OpenGl;
    settings.save();
}

/// After [`crate::run`] failed: if wgpu was being tried and never drew a frame, the failure was most likely wgpu's.
/// Saves OpenGL and starts the app again, so the user only sees the window open a little late. Returns whether it did.
#[cfg(not(target_os = "android"))]
pub fn relaunch_if_wgpu_failed() -> bool {
    let mut settings = Settings::load();
    if settings.renderer != Renderer::Modern || settings.wgpu != WgpuCheck::Trying {
        return false;
    }
    fall_back(&mut settings, "the app stopped before wgpu drew a frame");
    let relaunched = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn());
    if let Err(e) = &relaunched {
        tracing::warn!("couldn't start the app again: {e}");
    }
    relaunched.is_ok()
}

/// The graphics API Skia draws with, for the stats overlay; set each time the window's surface is made.
static RENDERER: Mutex<&str> = Mutex::new("—");

pub fn renderer_name() -> &'static str {
    RENDERER.lock().map_or("—", |name| *name)
}

/// Android: whether the window's surface went away since the app started, so the next one is made again; see
/// [`started`].
#[cfg(target_os = "android")]
static TORN_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "android")]
thread_local! {
    #[allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android")]
    static SETTINGS: RefCell<Option<Rc<RefCell<Settings>>>> = const { RefCell::new(None) };
}

/// Android: the activity starts again (back from the background). If its surface went away and Vulkan has only drawn
/// on a first one, the surface about to be made again is the test: a crash before it draws falls back next launch.
#[cfg(target_os = "android")]
pub fn started() {
    use std::sync::atomic::Ordering;
    if !TORN_DOWN.load(Ordering::Relaxed) {
        return;
    }
    SETTINGS.with_borrow(|settings| {
        let Some(mut settings) = settings.as_ref().and_then(|s| s.try_borrow_mut().ok()) else { return };
        if settings.renderer == Renderer::Modern && settings.wgpu == WgpuCheck::Drew {
            settings.wgpu = WgpuCheck::Trying;
            settings.save();
        }
    });
}

/// Notes which graphics API the window renders with, and marks wgpu as working here once it draws a frame.
pub fn watch_renderer(ui: &crate::ui::AppWindow, settings: &Rc<RefCell<Settings>>) {
    use slint::{ComponentHandle, GraphicsAPI, RenderingState};
    #[cfg(target_os = "android")]
    SETTINGS.set(Some(settings.clone()));
    let settings = settings.clone();
    let weak = ui.as_weak();
    // Surfaces made in this process: on Android, Vulkan works once it drew on one made again
    let mut setups = 0;
    // Every frame comes through here; only the first one on each surface matters
    let mut drew = false;
    let result = ui.window().set_rendering_notifier(move |state, api| match state {
        RenderingState::RenderingSetup => {
            let name = match api {
                GraphicsAPI::WGPU30 { device, queue, .. } => {
                    crate::gpu::start(device, queue);
                    match device.adapter_info().backend {
                        slint::wgpu_30::wgpu::Backend::Metal => "Metal",
                        slint::wgpu_30::wgpu::Backend::Dx12 => "D3D12",
                        slint::wgpu_30::wgpu::Backend::Vulkan => "Vulkan",
                        _ => "wgpu",
                    }
                }
                GraphicsAPI::NativeOpenGL { .. } => "OpenGL",
                _ => "other",
            };
            if let Ok(mut renderer) = RENDERER.lock() {
                *renderer = name;
            }
            tracing::info!("drawing with {name}");
            setups += 1;
            drew = false;
            // Only on Android, whose backend falls back by itself: wgpu was asked for and couldn't make the surface
            if name == "OpenGL"
                && let Ok(mut settings) = settings.try_borrow_mut()
                && settings.renderer == Renderer::Modern
            {
                fall_back(&mut settings, "wgpu couldn't make the window's surface");
                if let Some(ui) = weak.upgrade() {
                    ui.set_renderer("opengl".into());
                }
            }
        }
        RenderingState::RenderingTeardown => {
            crate::gpu::stop();
            if let Some(ui) = weak.upgrade() {
                crate::video::drop_textures(&ui);
            }
            #[cfg(target_os = "android")]
            TORN_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        RenderingState::AfterRendering if !drew => {
            drew = true;
            let Ok(mut settings) = settings.try_borrow_mut() else { return };
            if settings.wgpu == WgpuCheck::Trying && renderer_name() != "OpenGL" {
                settings.wgpu =
                    if cfg!(target_os = "android") && setups < 2 { WgpuCheck::Drew } else { WgpuCheck::Works };
                settings.save();
            }
        }
        _ => {}
    });
    if let Err(e) = result {
        tracing::warn!("can't watch the renderer: {e:?}");
    }
}

#[cfg(target_os = "macos")]
fn native_frame(attributes: WindowAttributes) -> WindowAttributes {
    use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
    attributes.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
}

#[cfg(not(any(target_os = "macos", target_os = "android")))]
fn native_frame(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_theme(Some(slint::winit_030::winit::window::Theme::Dark))
}

/// Reports to [`crate::video`] whether the window can be seen. macOS says when it's minimized or fully covered;
/// Windows only shrinks a minimized window to nothing.
#[cfg(not(target_os = "android"))]
pub fn watch_shown(ui: &crate::ui::AppWindow) {
    use slint::ComponentHandle;
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
    ui.window().on_winit_window_event(|_, event| {
        match event {
            WindowEvent::Occluded(occluded) => crate::video::set_window_shown(!occluded),
            #[cfg(target_os = "windows")]
            WindowEvent::Resized(size) => crate::video::set_window_shown(size.width > 0 && size.height > 0),
            _ => {}
        }
        EventResult::Propagate
    });
}
