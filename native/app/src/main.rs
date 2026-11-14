// No console window behind the app on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    // Velopack's install, uninstall and update hooks run here and may end the process, so before anything else
    velopack::VelopackApp::build().run();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    call_danya::window::select_backend().context("no usable graphics backend")?;
    let result = call_danya::run();
    if let Err(e) = &result
        && call_danya::window::relaunch_if_wgpu_failed()
    {
        tracing::warn!("restarted with OpenGL after: {e:#}");
        return Ok(());
    }
    result
}
