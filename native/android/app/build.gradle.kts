plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// The Rust workspace this project builds (`native/`), and the app's version, kept in its Cargo.toml like desktop's
val workspace: File = rootDir.parentFile
val appVersion: String = workspace.resolve("app/Cargo.toml").readLines()
    .first { it.startsWith("version = ") }.substringAfter('"').substringBefore('"')
val minApi = 29

android {
    namespace = "com.danendz.call"
    compileSdk = 35
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "com.danendz.call"
        minSdk = minApi
        targetSdk = 35
        versionName = appVersion
        // 1.2.3 → 10203, so every release's code is higher than the last
        versionCode = appVersion.split('.').fold(0) { code, part -> code * 100 + part.toInt() }
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release {
            // R8 would strip WebRTC's and Slint's Java classes, which are only reached from native code
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("rust/jniLibs"))
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

/** Builds the Rust app as `libcall_danya.so` with cargo-ndk, in the profile matching the Android build type. */
fun cargoTask(variant: String, release: Boolean) = tasks.register<Exec>("cargo$variant") {
    workingDir = workspace
    val sdk = android.sdkDirectory
    environment("ANDROID_NDK_HOME", android.ndkDirectory.absolutePath)
    environment("ANDROID_HOME", sdk.absolutePath)
    // Slint compiles a Java helper against it; cargo-ndk's API level would point it at a platform that may be missing
    environment("ANDROID_JAR", sdk.resolve("platforms/${android.compileSdkVersion}/android.jar").absolutePath)
    environment("JAVA_HOME", System.getProperty("java.home"))
    commandLine(
        listOf("cargo", "ndk", "-t", "arm64-v8a", "-P", "$minApi", "-o", layout.buildDirectory.dir("rust/jniLibs").get().asFile.path,
            "rustc", "-p", "call-danya", "--lib", "--crate-type", "cdylib", "--locked") +
            (if (release) listOf("--release") else emptyList())
    )
}

val cargoDebug = cargoTask("Debug", release = false)
val cargoRelease = cargoTask("Release", release = true)

tasks.configureEach {
    when (name) {
        "mergeDebugJniLibFolders" -> dependsOn(cargoDebug)
        "mergeReleaseJniLibFolders" -> dependsOn(cargoRelease)
        "compileDebugKotlin", "compileDebugJavaWithJavac" -> dependsOn(cargoDebug)
        "compileReleaseKotlin", "compileReleaseJavaWithJavac" -> dependsOn(cargoRelease)
    }
}

dependencies {
    // WebRTC's Java side (audio devices, codecs), from the prebuilt the Rust build used; build.rs puts it there
    debugImplementation(files(workspace.resolve("target/aarch64-linux-android/debug/libwebrtc.jar")))
    releaseImplementation(files(workspace.resolve("target/aarch64-linux-android/release/libwebrtc.jar")))
}
