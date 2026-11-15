//! Sets the platform cfgs and, with the `wasm-translate` feature, compiles Bergamot's WebAssembly (vendor/bergamot,
//! shared with the web app) to machine code for the target, so the app deserializes it at runtime instead of carrying
//! Cranelift, which also took ~370 MB of memory to compile it. Without it the app runs Bergamot built natively
//! (crates/bergamot).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    platform_cfgs();
    wasm()
}

#[cfg(not(feature = "wasm-translate"))]
fn wasm() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(feature = "wasm-translate")]
fn wasm() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;

    let wasm = PathBuf::from("../../../vendor/bergamot/bergamot.wasm");
    println!("cargo::rerun-if-changed={}", wasm.display());
    let target = std::env::var("TARGET")?;
    let mut config = wasmtime::Config::new();
    // Set even when building for this machine, so the code doesn't assume the build machine's CPU
    config.target(&target)?;
    config.wasm_threads(true);
    if target.starts_with("x86_64") {
        // SIMD needs SSE4.1; every x86-64 CPU from the last fifteen years has these
        for flag in ["has_sse3", "has_ssse3", "has_sse41", "has_sse42", "has_popcnt"] {
            // SAFETY: wasmtime checks the CPU has them when the app loads the code, and refuses it if not
            unsafe { config.cranelift_flag_enable(flag) };
        }
    }
    let engine = wasmtime::Engine::new(&config)?;
    let compiled = engine.precompile_module(&std::fs::read(&wasm)?)?;
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR")?).join("bergamot.cwasm"), compiled)?;
    Ok(())
}

/// `cfg(desktop)` on macOS and Windows, `cfg(mobile)` on Android and iOS, so platform code says which it means
/// instead of "not Android".
fn platform_cfgs() {
    println!("cargo::rustc-check-cfg=cfg(desktop, mobile)");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos" | "windows") => println!("cargo::rustc-cfg=desktop"),
        Ok("android" | "ios") => println!("cargo::rustc-cfg=mobile"),
        _ => {}
    }
}
