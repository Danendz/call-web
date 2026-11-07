fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("translations")
        // Strings are translated the same wherever they appear, so the .po has no per-component contexts
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        println!("cargo::error={e}");
    }
}
