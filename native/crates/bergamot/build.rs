//! Builds bergamot-translator from source: the release Firefox's WASM build (vendor/bergamot) comes from, fetched once
//! into `target/bergamot-src/`, patched for current toolchains, then built with CMake per target into
//! `target/bergamot-build/<target>/` and linked statically. The C shim (`cpp/shim.cpp`) is compiled into Bergamot's own
//! library target, so it gets that target's include paths and defines.
//!
//! Needs git, CMake and network on the first build. Builds for the app's targets: macOS and iOS (NEON, Accelerate),
//! Android (NEON, through the NDK cargo-ndk names) and Windows on x86-64 (intgemm's SSE to AVX-512, picked at runtime).

use std::path::{Path, PathBuf};
use std::process::Command;

/// mozilla/translations at the commit `vendor/bergamot/glue.js` names (`BERGAMOT_VERSION_FULL`, v0.6.0+eea6e5a8)
const REPO: &str = "https://github.com/mozilla/translations";
const COMMIT: &str = "eea6e5a80aa4ddd86d9cc35ce9a65b79aa3ab96d";
/// The submodules the inference engine builds from; the others are for training
const SUBMODULES: &[&str] = &[
    "inference/3rd_party/ssplit-cpp",
    "inference/marian-fork/src/3rd_party/sentencepiece",
    "inference/marian-fork/src/3rd_party/ruy",
    "inference/marian-fork/src/3rd_party/simd_utils",
    "inference/marian-fork/src/3rd_party/intgemm",
];

/// (file, from, to): fixes for toolchains newer than the release's
const PATCHES: &[(&str, &str, &str)] = &[
    // Clang 21 rejects an out-of-range enum value in a constexpr, which SentencePiece's trainer (linked by Marian) has
    (
        "inference/marian-fork/src/3rd_party/sentencepiece/src/trainer_interface.cc",
        "  constexpr unicode_script::ScriptType kAnyType =",
        "  const unicode_script::ScriptType kAnyType =",
    ),
    // Old zlib takes any Apple target for classic Mac OS and defines `fdopen` away, breaking the SDK's stdio.h
    (
        "inference/marian-fork/src/3rd_party/zlib/zutil.h",
        "#if defined(MACOS) || defined(TARGET_OS_MAC)\n",
        "#if defined(MACOS)\n",
    ),
    // New compilers bring new warnings, which shouldn't fail a dependency's build
    (
        "inference/marian-fork/CMakeLists.txt",
        "list(APPEND ALL_WARNINGS -Wall; -Werror; -Wextra;",
        "list(APPEND ALL_WARNINGS -Wall; -Wextra;",
    ),
    // Marian alone builds as C++11 while Bergamot builds as C++17 against Marian's headers; one standard for both
    ("inference/marian-fork/CMakeLists.txt", "set(CMAKE_CXX_STANDARD 11)", "set(CMAKE_CXX_STANDARD 17)"),
    (
        "inference/marian-fork/CMakeLists.txt",
        "    set(CMAKE_CXX_FLAGS                 \"-std=c++11 -pthread ",
        "    set(CMAKE_CXX_FLAGS                 \"-std=c++17 -pthread ",
    ),
    // iOS won't let an app map memory executable, so PCRE2's JIT would only fail there; its interpreter does the same
    (
        "inference/3rd_party/ssplit-cpp/cmake/FindPCRE2.cmake",
        "set(PCRE2_JIT_OPTION  \"-DPCRE2_SUPPORT_JIT=ON\")",
        "set(PCRE2_JIT_OPTION  \"-DPCRE2_SUPPORT_JIT=OFF\")",
    ),
    // Marian sets a process-wide terminate handler (and on Linux a SIGSEGV one) when it makes its loggers, which takes
    // the app's own crash handling over; its handler also throws, so on iOS freeing a translator ended the app
    ("inference/marian-fork/src/common/logging.cpp", "  setErrorHandlers();\n  return loggers;", "  return loggers;"),
    // A destructor that logs: on iOS the log call throws, which in a destructor ends the app as the translator is freed
    (
        "inference/marian-fork/src/graph/parameters.h",
        "    LOG(debug, \"Destroyed parameter object of type {}\", acceptedElementType_);\n",
        "",
    ),
    // MSVC's warnings as errors, as with -Werror above
    ("inference/marian-fork/CMakeLists.txt", "list(APPEND ALL_WARNINGS /WX; /W4;)", "list(APPEND ALL_WARNINGS /W4;)"),
    // MSVC's whole-program optimization leaves objects only the same compiler's linker can read, at link time; Rust
    // links them
    ("inference/CMakeLists.txt", "/MT /O2 ${INTRINSICS} /MP /GL /DNDEBUG", "/MT /O2 ${INTRINSICS} /MP /DNDEBUG"),
    (
        "inference/marian-fork/CMakeLists.txt",
        "/MT /O2 ${INTRINSICS} /MP /GL /DNDEBUG",
        "/MT /O2 ${INTRINSICS} /MP /DNDEBUG",
    ),
    // Without /GL there's nothing to link-time optimize, and llvm-lib (cargo-xwin) rejects the flag
    (
        "inference/CMakeLists.txt",
        "set(CMAKE_STATIC_LINKER_FLAGS      \"${CMAKE_STATIC_LINKER_FLAGS} /LTCG:incremental\")",
        "",
    ),
    (
        "inference/marian-fork/CMakeLists.txt",
        "set(CMAKE_STATIC_LINKER_FLAGS      \"${CMAKE_STATIC_LINKER_FLAGS} /LTCG:incremental\")",
        "",
    ),
    // PCRE2's headers expect its DLL on Windows unless told it's linked statically
    (
        "inference/3rd_party/ssplit-cpp/src/ssplit/regex.h",
        "#define PCRE2_CODE_UNIT_WIDTH 8\n",
        "#define PCRE2_CODE_UNIT_WIDTH 8\n#define PCRE2_STATIC\n",
    ),
    // PCRE2 is built apart, where Visual Studio builds Debug unless asked, against the DLL runtime unless asked
    (
        "inference/3rd_party/ssplit-cpp/cmake/FindPCRE2.cmake",
        "    BUILD_COMMAND ${CMAKE_COMMAND} --build <BINARY_DIR>\n",
        "    BUILD_COMMAND ${CMAKE_COMMAND} --build <BINARY_DIR> --config Release\n    INSTALL_COMMAND ${CMAKE_COMMAND} --install <BINARY_DIR> --config Release\n",
    ),
    (
        "inference/3rd_party/ssplit-cpp/cmake/FindPCRE2.cmake",
        "    ${PCRE2_JIT_OPTION}\n",
        "    ${PCRE2_JIT_OPTION}\n    -DPCRE2_STATIC_RUNTIME=ON\n",
    ),
    // Marian makes a ruy context for every matrix product, which sets up ruy's allocator and reads the CPU's features
    // each time; one per thread is reused instead. Contexts aren't shared between threads, and the translator's
    // lives on one
    (
        "inference/marian-fork/src/tensors/cpu/ruy_interface.h",
        "\n      ruy::Context context;\n",
        "\n      static thread_local ruy::Context context;\n",
    ),
    (
        "inference/marian-fork/src/tensors/cpu/prod_blas.h",
        "                    const int ldc) {\n  ruy::Context context;\n",
        "                    const int ldc) {\n  static thread_local ruy::Context context;\n",
    ),
    // Compiles our shim into Bergamot's library
    (
        "inference/src/translator/CMakeLists.txt",
        "if(COMPILE_WASM)\n  target_compile_definitions(bergamot-translator-source PUBLIC WASM)",
        "if(BERGAMOT_SHIM)\n  target_sources(bergamot-translator-source PRIVATE ${BERGAMOT_SHIM})\nendif()\n\nif(COMPILE_WASM)\n  target_compile_definitions(bergamot-translator-source PUBLIC WASM)",
    ),
];

fn main() {
    if let Err(e) = build() {
        println!("cargo::error=bergamot: {e}");
    }
}

fn build() -> Result<(), String> {
    let crate_dir = PathBuf::from(env("CARGO_MANIFEST_DIR")?);
    let shim = crate_dir.join("cpp/shim.cpp");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed={}", shim.display());
    println!("cargo::rerun-if-env-changed=RUSTC_WORKSPACE_WRAPPER");
    // Clippy only checks the Rust, which links nothing, so CI's iOS lint skips the C++ build of several minutes
    if std::env::var("RUSTC_WORKSPACE_WRAPPER").is_ok_and(|w| w.contains("clippy-driver")) {
        return Ok(());
    }

    let target = env("TARGET")?;
    let platform = match target.as_str() {
        "aarch64-apple-ios" => Platform::Ios("iphoneos"),
        "aarch64-apple-ios-sim" => Platform::Ios("iphonesimulator"),
        "aarch64-apple-darwin" => Platform::MacOs,
        "aarch64-linux-android" => Platform::Android,
        "x86_64-pc-windows-msvc" => Platform::Windows,
        other => return Err(format!("no build for {other} yet")),
    };

    let target_dir = target_dir()?;
    let source = fetch(&target_dir.join("bergamot-src").join(&COMMIT[..12]))?;
    let build = target_dir.join("bergamot-build").join(&target);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;

    let mut configure = cmake();
    configure.arg("-S").arg(source.join("inference")).arg("-B").arg(&build);
    match platform {
        Platform::Ios(sysroot) => {
            // Marian sets its own compile flags, so the toolchain file is how the target reaches it and PCRE2's
            // sub-build
            let toolchain = format!(
                "set(CMAKE_SYSTEM_NAME iOS)\nset(CMAKE_OSX_SYSROOT {sysroot})\nset(CMAKE_OSX_DEPLOYMENT_TARGET 17.0)\n\
                 # Left empty when cross-compiling, and ruy's cpuinfo then leaves out its ARM code\n\
                 set(CMAKE_SYSTEM_PROCESSOR arm64)\n\
                 # Executables in a dependency would otherwise be app bundles, which its install rules don't allow\n\
                 set(CMAKE_MACOSX_BUNDLE OFF)\n"
            );
            let toolchain_file = build.join("toolchain.cmake");
            write_if_changed(&toolchain_file, &toolchain)?;
            configure.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain_file.display()));
        }
        Platform::MacOs => {}
        Platform::Android => {
            // The NDK's own toolchain file; PCRE2's sub-build gets it too
            configure.args([
                format!("-DCMAKE_TOOLCHAIN_FILE={}", ndk_toolchain()?),
                "-DANDROID_ABI=arm64-v8a".into(),
                // The app's minSdk (android/app/build.gradle.kts)
                "-DANDROID_PLATFORM=android-29".into(),
                // The app's library carries its own C++ runtime, as WebRTC's prebuilt does
                "-DANDROID_STL=c++_static".into(),
            ]);
        }
        Platform::Windows => {
            configure.args([
                // Matches the app's static C runtime (+crt-static), which Bergamot's flags already ask for (/MT)
                "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
                // Float matrix products through ruy, as on Android: Marian otherwise wants MKL or OpenBLAS on x86
                "-DUSE_RUY=ON",
                "-DUSE_RUY_SGEMM=ON",
                "-DUSE_MKL=OFF",
                // Builds for SSE2 only (MSVC's default /arch); intgemm picks SSSE3, AVX2 or AVX-512 by the CPU at
                // runtime, so the app still starts on any x86-64 CPU
                "-DBUILD_ARCH=x86-64-v2",
            ]);
            // cargo-xwin's clang-cl toolchain when cross-compiling from macOS (see cmake()), which it names the way the
            // cmake crate reads it
            println!("cargo::rerun-if-env-changed=CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc");
            if let Ok(toolchain) = std::env::var("CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc") {
                configure.arg(format!("-DCMAKE_TOOLCHAIN_FILE={toolchain}"));
            }
        }
    }
    configure.args([
        "-DCMAKE_BUILD_TYPE=Release",
        "-DGIT_SUBMODULE=OFF",
        "-DCOMPILE_TESTS=OFF",
        "-DCOMPILE_UNIT_TESTS=OFF",
        "-DCOMPILE_LIBRARY_ONLY=ON",
        "-DUSE_STATIC_LIBS=ON",
        "-DSSPLIT_USE_INTERNAL_PCRE2=ON",
    ]);
    if !matches!(platform, Platform::Windows) {
        configure.args([
            "-DBUILD_ARCH=armv8-a",
            // Marian defines these, which turn its SSE code into NEON through simd_utils, only for its own sources;
            // Bergamot's include the same headers
            "-DCMAKE_CXX_FLAGS=-DARM -DFMA -DSSE",
        ]);
    }
    configure.arg(format!("-DBERGAMOT_SHIM={}", shim.display()));
    run(&mut configure)?;

    let jobs = std::env::var("NUM_JOBS").unwrap_or_else(|_| "8".into());
    run(cmake().arg("--build").arg(&build).args([
        "--target",
        "bergamot-translator-source",
        "--config",
        "Release",
        "-j",
        &jobs,
    ]))?;

    let out = PathBuf::from(env("OUT_DIR")?);
    let windows = matches!(platform, Platform::Windows);
    for lib in archives(&build, if windows { "lib" } else { "a" })? {
        let Some(name) = lib.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
        let lib = if name.ends_with("sentencepiece_train") && !windows { without_protobuf(&lib, &out)? } else { lib };
        let Some(dir) = lib.parent() else { continue };
        println!("cargo::rustc-link-search=native={}", dir.display());
        println!("cargo::rustc-link-lib=static={}", if windows { &name } else { name.trim_start_matches("lib") });
    }
    match platform {
        Platform::Ios(_) | Platform::MacOs => {
            println!("cargo::rustc-link-lib=framework=Accelerate");
            println!("cargo::rustc-link-lib=c++");
        }
        Platform::Android => link_ndk_cxx(&ndk_toolchain()?)?,
        // Marian's file paths (pathie-cpp)
        Platform::Windows => println!("cargo::rustc-link-lib=shlwapi"),
    }
    Ok(())
}

/// The NDK's CMake toolchain file, which cargo-ndk names
fn ndk_toolchain() -> Result<String, String> {
    println!("cargo::rerun-if-env-changed=CARGO_NDK_CMAKE_TOOLCHAIN_PATH");
    std::env::var("CARGO_NDK_CMAKE_TOOLCHAIN_PATH")
        .or_else(|_| env("ANDROID_NDK_HOME").map(|ndk| format!("{ndk}/build/cmake/android.toolchain.cmake")))
}

/// Links the NDK's static C++ runtime, which the build compiled against. Only its two archives are copied out: the folder
/// they're in also has a static libc, which the linker would then take over the shared one.
fn link_ndk_cxx(toolchain: &str) -> Result<(), String> {
    let ndk = Path::new(toolchain).ancestors().nth(3).ok_or("the NDK's toolchain file isn't in an NDK")?;
    let prebuilt = ndk.join("toolchains/llvm/prebuilt");
    let host = std::fs::read_dir(&prebuilt)
        .map_err(|e| format!("{}: {e}", prebuilt.display()))?
        .flatten()
        .next()
        .ok_or_else(|| format!("{} is empty", prebuilt.display()))?;
    let libs = host.path().join("sysroot/usr/lib/aarch64-linux-android");
    let out = PathBuf::from(env("OUT_DIR")?).join("ndk-cxx");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for name in ["c++_static", "c++abi"] {
        let file = format!("lib{name}.a");
        std::fs::copy(libs.join(&file), out.join(&file)).map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rustc-link-lib=static={name}");
    }
    println!("cargo::rustc-link-search=native={}", out.display());
    Ok(())
}

#[derive(Clone, Copy)]
enum Platform {
    /// With the SDK's name
    Ios(&'static str),
    MacOs,
    Android,
    Windows,
}

/// Clones the source at `COMMIT` with the submodules it needs and patches it, once.
fn fetch(dir: &Path) -> Result<PathBuf, String> {
    let done = dir.join(".fetched");
    if !done.exists() {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        // Line endings as committed, also where git would turn them into CRLF (Windows), so the patches match
        run(Command::new("git")
            .args(["-c", "core.autocrlf=false", "clone", "--quiet", "--filter=blob:none", "--no-checkout", REPO])
            .arg(dir))?;
        run(Command::new("git").arg("-C").arg(dir).args(["-c", "core.autocrlf=false", "checkout", "--quiet", COMMIT]))?;
        run(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "core.autocrlf=false", "submodule", "update", "--init", "--recursive", "--depth", "1", "--"])
            .args(SUBMODULES))?;
        std::fs::write(&done, COMMIT).map_err(|e| format!("{}: {e}", done.display()))?;
    }
    for (file, from, to) in PATCHES {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !text.contains(from) {
            if to.is_empty() || text.contains(to) {
                continue;
            }
            return Err(format!("{file} no longer has what its patch replaces"));
        }
        write_if_changed(&path, &text.replacen(from, to, 1))?;
    }
    Ok(dir.to_path_buf())
}

/// A copy of SentencePiece's trainer library without the protobuf objects SentencePiece's own library also has, which
/// would otherwise warn of duplicate symbols on every link
fn without_protobuf(lib: &Path, out: &Path) -> Result<PathBuf, String> {
    let Some(main) = lib.parent().map(|dir| dir.join("libsentencepiece.a")) else { return Ok(lib.to_path_buf()) };
    let members = |archive: &Path| -> Result<Vec<String>, String> {
        let listed = Command::new("ar").arg("t").arg(archive).output().map_err(|e| format!("ar: {e}"))?;
        Ok(String::from_utf8_lossy(&listed.stdout).lines().map(str::to_owned).collect())
    };
    let shared = members(&main)?;
    let repeated: Vec<String> = members(lib)?.into_iter().filter(|m| m.ends_with(".o") && shared.contains(m)).collect();
    let copy = out.join("libsentencepiece_train.a");
    std::fs::copy(lib, &copy).map_err(|e| format!("{}: {e}", copy.display()))?;
    if !repeated.is_empty() {
        run(Command::new("ar").arg("d").arg(&copy).args(&repeated))?;
    }
    Ok(copy)
}

/// Every static library the build made, by its extension
fn archives(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == extension) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// Cargo's target folder. OUT_DIR is `<target>[/<triple>]/<profile>/build/<crate>-<hash>/out`.
fn target_dir() -> Result<PathBuf, String> {
    let out = PathBuf::from(env("OUT_DIR")?);
    let profile = out.ancestors().nth(3).ok_or("OUT_DIR is too short")?;
    let above = profile.parent().ok_or("OUT_DIR is too short")?;
    let cross = above.file_name().is_some_and(|name| name.to_str() == std::env::var("TARGET").ok().as_deref());
    let target = if cross { above.parent().ok_or("OUT_DIR is too short")? } else { above };
    Ok(target.to_path_buf())
}

fn cmake() -> Command {
    let mut command = Command::new("cmake");
    // The release's dependencies ask for CMake versions CMake 4 no longer supports
    command.env("CMAKE_POLICY_VERSION_MINIMUM", "3.5");
    // .cargo/config.toml sets it for iOS builds, and clang then builds for iOS whenever nothing says otherwise, as
    // with macOS; the toolchain file gives iOS builds their own
    command.env_remove("IPHONEOS_DEPLOYMENT_TARGET");
    // cargo-xwin, cross-compiling for Windows from macOS, hands clang-cl the target and the Windows headers in
    // CMAKE_C(XX)_FLAGS, which Marian replaces with its own; clang-cl reads CL too, as cl does. MSVC also compiles
    // intrinsics without their header or their instruction set turned on, which clang-cl has to be given: Marian's x86
    // code uses SSE4.1, and /arch:SSE2 (MSVC's default, which clang-cl warns about on x64) is an error under /WX
    println!("cargo::rerun-if-env-changed=CFLAGS_x86_64_pc_windows_msvc");
    if let Ok(flags) = std::env::var("CFLAGS_x86_64_pc_windows_msvc") {
        command.env("CL", format!("{flags} -msse4.1 /FIintrin.h -Wno-unused-command-line-argument"));
    }
    command
}

fn run(command: &mut Command) -> Result<(), String> {
    // Cargo reads a build script's stdout for its instructions
    command.stdout(std::io::stderr());
    let status = command.status().map_err(|e| format!("{command:?}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{command:?} failed: {status}")) }
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} isn't set"))
}
