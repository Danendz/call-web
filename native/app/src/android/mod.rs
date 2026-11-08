//! The Android entry points. `MainActivity` (android/) loads this library and calls `nativeInit` on its main thread,
//! then Android's `NativeActivity` starts [`android_main`] on a thread of its own, where the app runs as on desktop.

mod log;

use jni::JNIEnv;
use jni::objects::{JClass, JObject};

/// `MainActivity.nativeInit(context)`, on the activity's main thread before the app starts.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_MainActivity_nativeInit(env: JNIEnv, _class: JClass, context: JObject) {
    log::init();
    match env.get_java_vm() {
        Ok(vm) => {
            if !engine::initialize_android_context(&vm, &context) {
                tracing::error!("WebRTC couldn't start on Android");
            }
        }
        Err(e) => tracing::error!("no JavaVM: {e}"),
    }
}

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    if let Some(dir) = app.internal_data_path() {
        let _ = crate::settings::ANDROID_DIR.set(dir);
    }
    if let Err(e) = slint::android::init(app) {
        tracing::error!("no usable graphics backend: {e}");
        return;
    }
    if let Err(e) = crate::run() {
        tracing::error!("{e:#}");
    }
}
