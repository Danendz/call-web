//! Picks Slint's backend and renderer and tweaks the native window: on macOS the content runs under a transparent
//! title bar, on Windows the frame is dark. Also tells video when the window can't be seen. Uses Slint's unstable
//! winit hooks, which is why slint is pinned exactly.
//!
//! Skia draws over wgpu (Metal, D3D12 or Vulkan) unless Settings says OpenGL. wgpu that can't start, or that
//! never drew a frame at the last launch, falls back to OpenGL and stays there, so a bad driver costs at most one
//! relaunch.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use slint::winit_030::winit::window::WindowAttributes;

use crate::settings::{Renderer, Settings, WgpuCheck};

pub fn select_backend() -> Result<(), slint::PlatformError> {
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

/// In a debug build, `CALL_FAKE_NO_WGPU=1` offers wgpu no backends, so the fallback to OpenGL can be tested on a
/// machine where wgpu works.
fn wgpu_configuration() -> slint::wgpu_30::WGPUConfiguration {
    use slint::wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu};
    if cfg!(debug_assertions) && std::env::var_os("CALL_FAKE_NO_WGPU").is_some() {
        let mut settings = WGPUSettings::default();
        settings.backends = wgpu::Backends::empty();
        return WGPUConfiguration::Automatic(settings);
    }
    WGPUConfiguration::default()
}

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

/// The graphics API Skia draws with, for the stats overlay; set once the window first renders.
static RENDERER: OnceLock<&'static str> = OnceLock::new();

pub fn renderer_name() -> &'static str {
    RENDERER.get().copied().unwrap_or("—")
}

/// Notes which graphics API the window renders with, hands its device to video, and marks wgpu as working here once
/// it draws a frame.
pub fn watch_renderer(ui: &crate::ui::AppWindow, settings: &Rc<RefCell<Settings>>) {
    use slint::{ComponentHandle, GraphicsAPI, RenderingState};
    let settings = settings.clone();
    // Every frame comes through here; only the first one matters
    let mut drew = false;
    let result = ui.window().set_rendering_notifier(move |state, api| match state {
        RenderingState::RenderingSetup => {
            use slint::wgpu_30::wgpu::Backend;
            let name = match api {
                GraphicsAPI::WGPU30 { device, queue, .. } => {
                    let backend = device.adapter_info().backend;
                    // Video converts on the GPU where its submits can run beside Skia's; see video::gpu
                    if matches!(backend, Backend::Metal | Backend::Dx12) {
                        crate::video::gpu::set_device(Some((device.clone(), queue.clone())));
                    }
                    match backend {
                        Backend::Metal => "Metal",
                        Backend::Dx12 => "D3D12",
                        Backend::Vulkan => "Vulkan",
                        _ => "wgpu",
                    }
                }
                GraphicsAPI::NativeOpenGL { .. } => "OpenGL",
                _ => "other",
            };
            let _ = RENDERER.set(name);
            tracing::info!("drawing with {name}");
        }
        RenderingState::RenderingTeardown => crate::video::gpu::set_device(None),
        RenderingState::AfterRendering if !drew => {
            drew = true;
            let Ok(mut settings) = settings.try_borrow_mut() else { return };
            if settings.wgpu == WgpuCheck::Trying && RENDERER.get().is_some_and(|name| *name != "OpenGL") {
                settings.wgpu = WgpuCheck::Works;
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

#[cfg(not(target_os = "macos"))]
fn native_frame(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_theme(Some(slint::winit_030::winit::window::Theme::Dark))
}

/// Reports to [`crate::video`] whether the window can be seen. macOS says when it's minimized or fully covered;
/// Windows only shrinks a minimized window to nothing.
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
