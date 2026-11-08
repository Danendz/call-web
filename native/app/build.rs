fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("translations")
        // Strings are translated the same wherever they appear, so the .po has no per-component contexts
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        println!("cargo::error={e}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        android();
    }
}

/// WebRTC's Java classes call into its native code through `Java_livekit_org_webrtc_*` symbols, which only the final
/// library can export: webrtc-sys's link args don't reach us. Its `libwebrtc.jar` goes next to the library
/// (`target/<triple>/<profile>/`), where the Gradle build picks it up.
fn android() {
    if let Err(e) = webrtc_sys_build::configure_jni_symbols() {
        println!("cargo::error=couldn't export WebRTC's JNI symbols: {e:#}");
    }
    let jar = webrtc_sys_build::webrtc_dir().join("libwebrtc.jar");
    let out = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from);
    // OUT_DIR is target/<triple>/<profile>/build/<crate>/out
    let Some(dest) = out.as_deref().and_then(|out| out.ancestors().nth(3)) else { return };
    if let Err(e) = std::fs::copy(&jar, dest.join("libwebrtc.jar")) {
        println!("cargo::error=couldn't copy {}: {e}", jar.display());
    }
}
