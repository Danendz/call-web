//! Picks Slint's backend and tweaks the native window: on macOS the content runs under a transparent title bar,
//! on Windows the frame is dark. Uses Slint's unstable winit hook, which is why slint is pinned exactly.

use slint::winit_030::winit::window::WindowAttributes;

pub fn select_backend() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("skia-opengl".into())
        .with_winit_window_attributes_hook(native_frame)
        .select()
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
