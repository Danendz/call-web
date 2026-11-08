//! The Android entry points. `MainActivity` (android/) loads this library and calls `nativeInit` on its main thread,
//! then Android's `NativeActivity` starts [`android_main`] on a thread of its own, where the app runs as on desktop.
//! What only Java can do (the call's foreground service and notification, audio routing, settings pages) is in
//! `Shim.kt`, called through [`Java`]; what Android reports back arrives in the `Shim_native*` functions.

mod log;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::ui::{AppPhase, AppWindow};

// Must match Shim.kt
const ACTION_TOGGLE_MIC: jint = 1;
const ACTION_HANG_UP: jint = 2;
const ACTION_INTERRUPTED: jint = 3;
const ACTION_RESUMED: jint = 4;
/// Indexed by Shim.kt's route numbers
const ROUTES: [&str; 3] = ["speaker", "earpiece", "bluetooth"];

/// What Rust needs to call into Java from its own threads: Shim's class must be found on the main thread, whose
/// class loader knows the app's classes.
struct Java {
    vm: JavaVM,
    shim: GlobalRef,
    context: GlobalRef,
}

static JAVA: OnceLock<Java> = OnceLock::new();

thread_local! {
    // The window, for what Android reports; set on the UI thread, where those reports are handled
    #[allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android")]
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    // The mic was turned off for a phone call, and comes back on after it
    #[allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android")]
    static MUTED_FOR_CALL: Cell<bool> = const { Cell::new(false) };
    // What the call notification last showed: in a call, mic on, its title, text and labels
    #[allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android")]
    static SHOWN: RefCell<Option<(bool, bool, Vec<SharedString>)>> = const { RefCell::new(None) };
}

/// `MainActivity.nativeInit(context)`, on the activity's main thread before the app starts.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_MainActivity_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    log::init();
    let vm = match env.get_java_vm() {
        Ok(vm) => vm,
        Err(e) => return tracing::error!("no JavaVM: {e}"),
    };
    if !engine::initialize_android_context(&vm, &context) {
        tracing::error!("WebRTC couldn't start on Android");
    }
    match remember(&mut env, &context) {
        Ok(java) => {
            let _ = JAVA.set(java);
        }
        Err(e) => tracing::error!("Shim isn't reachable: {e}"),
    }
}

fn remember(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<Java> {
    let shim = env.find_class("com/danendz/call/Shim")?;
    Ok(Java { vm: env.get_java_vm()?, shim: env.new_global_ref(shim)?, context: env.new_global_ref(context)? })
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

/// Runs `f` with a JNI env on this thread, Shim's class and the app context. A failure, Java exception included,
/// is logged: none of these calls is worth stopping the app for.
fn with_java(f: impl FnOnce(&mut JNIEnv, &JClass, &JObject) -> jni::errors::Result<()>) {
    let Some(java) = JAVA.get() else { return };
    let result = java.vm.attach_current_thread_permanently().and_then(|mut env| {
        let result = f(&mut env, <&JClass>::from(java.shim.as_obj()), java.context.as_obj());
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    });
    if let Err(e) = result {
        tracing::warn!("call into Android failed: {e}");
    }
}

/// Takes in the window, and wires up what Android does for it.
pub(crate) fn attach(ui: &AppWindow, settings: &Rc<RefCell<crate::settings::Settings>>) {
    UI.with(|cell| *cell.borrow_mut() = Some(ui.as_weak()));

    ui.on_pick_output(|id| {
        if let Some(route) = ROUTES.iter().position(|r| *r == id.as_str()) {
            with_java(|env, shim, context| {
                env.call_static_method(
                    shim,
                    "setRoute",
                    "(Landroid/content/Context;I)V",
                    &[JValue::Object(context), JValue::Int(route as jint)],
                )
                .map(drop)
            });
        }
    });

    if !settings.borrow().background_guide_seen {
        let mut vendor = String::new();
        with_java(|env, shim, _| {
            let name = env.call_static_method(shim, "vendor", "()Ljava/lang/String;", &[])?.l()?;
            vendor = env.get_string(&JString::from(name))?.into();
            Ok(())
        });
        ui.set_background_guide(if matches!(vendor.as_str(), "huawei" | "honor") { "huawei" } else { "other" }.into());
    }
    let seen = {
        let settings = settings.clone();
        let weak = ui.as_weak();
        move || {
            let mut settings = settings.borrow_mut();
            settings.background_guide_seen = true;
            settings.save();
            if let Some(ui) = weak.upgrade() {
                ui.set_background_guide(SharedString::new());
            }
        }
    };
    ui.on_dismiss_background_guide(seen.clone());
    ui.on_open_background_settings(move || {
        with_java(|env, shim, context| {
            env.call_static_method(
                shim,
                "openBackgroundSettings",
                "(Landroid/content/Context;)V",
                &[JValue::Object(context)],
            )
            .map(drop)
        });
        seen();
    });
}

/// Keeps the call's foreground service and notification in step with the window; called on the UI's clock.
pub(crate) fn sync(ui: &AppWindow) {
    let phase = ui.get_phase();
    let in_call = matches!(phase, AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall);
    let mic_on = ui.get_mic_on();
    let notice = ui.get_call_notice();
    // Title while waiting or in the call, then the text and the actions' labels
    let title = notice.row_data(if phase == AppPhase::InCall { 1 } else { 0 });
    let labels: Vec<SharedString> = title.into_iter().chain(notice.iter().skip(2)).collect();
    let state = (in_call, mic_on, labels);
    if SHOWN.with(|shown| shown.borrow().as_ref() == Some(&state)) {
        return;
    }
    let (_, _, labels) = &state;
    with_java(|env, shim, context| {
        let array = env.new_object_array(labels.len() as jint, "java/lang/String", JObject::null())?;
        for (i, label) in labels.iter().enumerate() {
            let label = env.new_string(label)?;
            env.set_object_array_element(&array, i as jint, label)?;
        }
        env.call_static_method(
            shim,
            "callState",
            "(Landroid/content/Context;ZZ[Ljava/lang/String;)V",
            &[
                JValue::Object(context),
                JValue::Bool(in_call.into()),
                JValue::Bool(mic_on.into()),
                JValue::Object(&array),
            ],
        )
        .map(drop)
    });
    if !in_call {
        MUTED_FOR_CALL.set(false);
    }
    SHOWN.with(|shown| *shown.borrow_mut() = Some(state));
}

fn on_ui(f: impl FnOnce(&AppWindow) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = UI.with(|cell| cell.borrow().as_ref().and_then(slint::Weak::upgrade)) {
            f(&ui);
        }
    });
}

/// `Shim.nativeAction`: the notification's buttons, and a phone call taking or giving back the audio.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeAction(_env: JNIEnv, _class: JClass, action: jint) {
    on_ui(move |ui| match action {
        ACTION_TOGGLE_MIC => ui.invoke_toggle_mic(),
        ACTION_HANG_UP => ui.invoke_hang_up(),
        ACTION_INTERRUPTED if ui.get_mic_on() && !MUTED_FOR_CALL.get() => {
            MUTED_FOR_CALL.set(true);
            ui.invoke_toggle_mic();
        }
        ACTION_RESUMED if MUTED_FOR_CALL.replace(false) && !ui.get_mic_on() => ui.invoke_toggle_mic(),
        _ => {}
    });
}

/// `Shim.nativeAudio`: which outputs there are, as bits of `1 << route`, and the one the call plays on.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeAudio(
    _env: JNIEnv,
    _class: JClass,
    available: jint,
    route: jint,
) {
    on_ui(move |ui| {
        // One output is no choice
        let mut outputs: Vec<SharedString> =
            ROUTES.iter().enumerate().filter(|(i, _)| available & (1 << i) != 0).map(|(_, r)| (*r).into()).collect();
        if outputs.len() < 2 {
            outputs.clear();
        }
        ui.set_outputs(ModelRc::new(VecModel::from(outputs)));
        let current = usize::try_from(route).ok().and_then(|i| ROUTES.get(i)).copied().unwrap_or_default();
        ui.set_output(current.into());
    });
}
