//! Builds bergamot-translator from source: the release Firefox's WASM build (vendor/bergamot) comes from, fetched once
//! into `target/bergamot-src/`, patched for current Apple toolchains, then built with CMake per target into
//! `target/bergamot-build/<target>/` and linked statically. The C shim (`cpp/shim.cpp`) is compiled into Bergamot's own
//! library target, so it gets that target's include paths and defines.
//!
//! Needs git, CMake and network on the first build; Apple targets only so far (iOS, its simulator, macOS on arm64).

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

    let target = env("TARGET")?;
    let platform = match target.as_str() {
        "aarch64-apple-ios" => Some(("iphoneos", "iOS")),
        "aarch64-apple-ios-sim" => Some(("iphonesimulator", "iOS")),
        "aarch64-apple-darwin" => None,
        other => return Err(format!("no build for {other} yet")),
    };

    let target_dir = target_dir()?;
    let source = fetch(&target_dir.join("bergamot-src").join(&COMMIT[..12]))?;
    let build = target_dir.join("bergamot-build").join(&target);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;

    // Marian sets its own compile flags, so the toolchain file is how the target reaches it and PCRE2's sub-build
    let mut toolchain = String::new();
    if let Some((sysroot, system)) = platform {
        toolchain = format!(
            "set(CMAKE_SYSTEM_NAME {system})\nset(CMAKE_OSX_SYSROOT {sysroot})\nset(CMAKE_OSX_DEPLOYMENT_TARGET 17.0)\n\
             # Left empty when cross-compiling, and ruy's cpuinfo then leaves out its ARM code\n\
             set(CMAKE_SYSTEM_PROCESSOR arm64)\n\
             # Executables in a dependency would otherwise be app bundles, which its install rules don't allow\n\
             set(CMAKE_MACOSX_BUNDLE OFF)\n"
        );
    }
    let toolchain_file = build.join("toolchain.cmake");
    write_if_changed(&toolchain_file, &toolchain)?;

    let mut configure = cmake();
    configure.arg("-S").arg(source.join("inference")).arg("-B").arg(&build);
    if platform.is_some() {
        configure.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain_file.display()));
    }
    configure.args([
        "-DCMAKE_BUILD_TYPE=Release",
        "-DBUILD_ARCH=armv8-a",
        "-DGIT_SUBMODULE=OFF",
        "-DCOMPILE_TESTS=OFF",
        "-DCOMPILE_UNIT_TESTS=OFF",
        "-DCOMPILE_LIBRARY_ONLY=ON",
        "-DUSE_STATIC_LIBS=ON",
        "-DSSPLIT_USE_INTERNAL_PCRE2=ON",
        // Marian defines these, which turn its SSE code into NEON through simd_utils, only for its own sources;
        // Bergamot's include the same headers
        "-DCMAKE_CXX_FLAGS=-DARM -DFMA -DSSE",
    ]);
    configure.arg(format!("-DBERGAMOT_SHIM={}", shim.display()));
    run(&mut configure)?;

    let jobs = std::env::var("NUM_JOBS").unwrap_or_else(|_| "8".into());
    run(cmake().arg("--build").arg(&build).args(["--target", "bergamot-translator-source", "-j", &jobs]))?;

    let out = PathBuf::from(env("OUT_DIR")?);
    for lib in archives(&build)? {
        let lib = if lib.ends_with("libsentencepiece_train.a") { without_protobuf(&lib, &out)? } else { lib };
        let (Some(dir), Some(name)) = (lib.parent(), lib.file_stem().and_then(|s| s.to_str())) else { continue };
        println!("cargo::rustc-link-search=native={}", dir.display());
        println!("cargo::rustc-link-lib=static={}", name.trim_start_matches("lib"));
    }
    println!("cargo::rustc-link-lib=framework=Accelerate");
    println!("cargo::rustc-link-lib=c++");
    Ok(())
}

/// Clones the source at `COMMIT` with the submodules it needs and patches it, once.
fn fetch(dir: &Path) -> Result<PathBuf, String> {
    let done = dir.join(".fetched");
    if !done.exists() {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        run(Command::new("git").args(["clone", "--quiet", "--filter=blob:none", "--no-checkout", REPO]).arg(dir))?;
        run(Command::new("git").arg("-C").arg(dir).args(["checkout", "--quiet", COMMIT]))?;
        run(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["submodule", "update", "--init", "--recursive", "--depth", "1", "--"])
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

/// Every static library the build made
fn archives(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "a") {
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
