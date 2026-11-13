//! The Android entry points. `MainActivity` (android/) loads this library and calls `nativeInit` on its main thread,
//! then Android's `NativeActivity` starts [`android_main`] on a thread of its own, where the app runs as on desktop.
//! What only Java can do (the call's foreground service and notification, audio routing, settings pages) is in
//! `Shim.kt`, called through [`Java`]; what Android reports back arrives in the `Shim_native*` functions.

mod fonts;
mod log;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JByteBuffer, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jint};
use jni::{JNIEnv, JavaVM};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::ui::{AppPhase, AppWindow};

// Must match Shim.kt
const ACTION_TOGGLE_MIC: jint = 1;
const ACTION_HANG_UP: jint = 2;
const ACTION_INTERRUPTED: jint = 3;
const ACTION_RESUMED: jint = 4;
/// Indexed by Shim.kt's route numbers
const ROUTES: [&str; 4] = ["speaker", "earpiece", "bluetooth", "headset"];

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
    // What Android was last told: in a call, mic on, the notification's title, text and labels, the main video's size
    #[allow(clippy::missing_const_for_thread_local, reason = "it is const; misfires on Android")]
    static SHOWN: RefCell<Option<CallState>> = const { RefCell::new(None) };
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
            engine::android_camera::set_backend(Box::new(Camera));
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
    fonts::init();
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

/// Adds a JPEG to the phone's gallery as `Pictures/Call Danya/<name>.jpg`; its content URI, or None if it couldn't.
pub(crate) fn save_picture(name: &str, jpeg: &[u8]) -> Option<String> {
    let mut uri = None;
    with_java(|env, shim, context| {
        let name = env.new_string(name)?;
        let bytes = env.byte_array_from_slice(jpeg)?;
        let saved = env
            .call_static_method(
                shim,
                "savePicture",
                "(Landroid/content/Context;Ljava/lang/String;[B)Ljava/lang/String;",
                &[JValue::Object(context), JValue::Object(&name), JValue::Object(&bytes)],
            )?
            .l()?;
        if !saved.is_null() {
            uri = Some(env.get_string(&JString::from(saved))?.into());
        }
        Ok(())
    });
    uri
}

/// Opens a picture [`save_picture`] saved, in the phone's photo viewer.
pub(crate) fn open_picture(uri: &str) {
    with_java(|env, shim, context| {
        let uri = env.new_string(uri)?;
        env.call_static_method(
            shim,
            "openPicture",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(context), JValue::Object(&uri)],
        )
        .map(drop)
    });
}

/// A short buzz, for the other person's reaction.
pub(crate) fn vibrate(ms: i64) {
    with_java(|env, shim, context| {
        env.call_static_method(
            shim,
            "vibrate",
            "(Landroid/content/Context;J)V",
            &[JValue::Object(context), JValue::Long(ms)],
        )
        .map(drop)
    });
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

/// Checks `{server}/updates/android.json` (`Updater.kt`) and installs from the start screen's button. A debug build
/// talks to the dev server, which has no updates.
pub(crate) fn updates(ui: &AppWindow, server: &url::Url) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(feed) = server.join("updates") else { return };
    with_java(|env, shim, context| {
        let feed = env.new_string(feed.as_str())?;
        env.call_static_method(
            shim,
            "startUpdates",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(context), JValue::Object(&feed)],
        )
        .map(drop)
    });
    ui.on_update_now(|| {
        with_java(|env, shim, context| {
            env.call_static_method(shim, "installUpdate", "(Landroid/content/Context;)V", &[JValue::Object(context)])
                .map(drop)
        });
    });
}

type CallState = (bool, bool, Vec<SharedString>, (u32, u32));

/// Keeps the call's foreground service, notification and picture-in-picture shape in step with the window; called on
/// the UI's clock.
pub(crate) fn sync(ui: &AppWindow) {
    let phase = ui.get_phase();
    let in_call = matches!(phase, AppPhase::Starting | AppPhase::Waiting | AppPhase::InCall);
    let mic_on = ui.get_mic_on();
    let notice = ui.get_call_notice();
    // Title while waiting or in the call, then the text and the actions' labels
    let title = notice.row_data(if phase == AppPhase::InCall { 1 } else { 0 });
    let labels: Vec<SharedString> = title.into_iter().chain(notice.iter().skip(2)).collect();
    // The main video, which picture-in-picture shows: their screen when shared, else their camera turned upright,
    // else yours while they're not there
    let (screen, peer, own) = (ui.get_screen_video().size(), ui.get_peer_video().size(), ui.get_self_video().size());
    let video = if ui.get_remote_screen() && screen.width > 0 {
        (screen.width, screen.height)
    } else if peer.width > 0 {
        if ui.get_peer_frame_rotation() % 180 == 0 { (peer.width, peer.height) } else { (peer.height, peer.width) }
    } else {
        (own.width, own.height)
    };
    let state = (in_call, mic_on, labels, ratio(video));
    if SHOWN.with(|shown| shown.borrow().as_ref() == Some(&state)) {
        return;
    }
    let (_, _, labels, (width, height)) = &state;
    with_java(|env, shim, context| {
        let array = env.new_object_array(labels.len() as jint, "java/lang/String", JObject::null())?;
        for (i, label) in labels.iter().enumerate() {
            let label = env.new_string(label)?;
            env.set_object_array_element(&array, i as jint, label)?;
        }
        env.call_static_method(
            shim,
            "callState",
            "(Landroid/content/Context;ZZ[Ljava/lang/String;II)V",
            &[
                JValue::Object(context),
                JValue::Bool(in_call.into()),
                JValue::Bool(mic_on.into()),
                JValue::Object(&array),
                JValue::Int(*width as jint),
                JValue::Int(*height as jint),
            ],
        )
        .map(drop)
    });
    if !in_call {
        MUTED_FOR_CALL.set(false);
    }
    SHOWN.with(|shown| *shown.borrow_mut() = Some(state));
}

/// As a reduced ratio, so the video stepping its resolution up or down doesn't count as a change.
fn ratio((w, h): (u32, u32)) -> (u32, u32) {
    let (mut a, mut b) = (w, h);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    w.checked_div(a).zip(h.checked_div(a)).unwrap_or_default()
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

/// `Updater.nativeUpdateProgress`: how much of an update has downloaded, or -1 when it stopped.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Updater_nativeUpdateProgress(_env: JNIEnv, _class: JClass, percent: jint) {
    on_ui(move |ui| ui.set_update_progress(percent));
}

/// `Updater.nativeUpdateReady`: an update is downloaded and checked, ready to install.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Updater_nativeUpdateReady(
    mut env: JNIEnv,
    _class: JClass,
    version: JString,
) {
    let Ok(version) = env.get_string(&version).map(String::from) else { return };
    on_ui(move |ui| ui.set_update_version(version.into()));
}

/// `Shim.nativePip`: the activity went into or out of picture-in-picture.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativePip(_env: JNIEnv, _class: JClass, pip: jboolean) {
    on_ui(move |ui| ui.set_pip(pip != 0));
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

/// The engine's camera, opened by `Camera.kt` through Shim.
struct Camera;

impl engine::android_camera::Backend for Camera {
    fn request_access(&self) {
        with_java(|env, shim, context| {
            env.call_static_method(shim, "requestCamera", "(Landroid/content/Context;)V", &[JValue::Object(context)])
                .map(drop)
        });
    }

    fn list(&self) -> Vec<engine::Device> {
        let mut devices = Vec::new();
        with_java(|env, shim, context| {
            let list = env
                .call_static_method(
                    shim,
                    "cameraList",
                    "(Landroid/content/Context;)[Ljava/lang/String;",
                    &[JValue::Object(context)],
                )?
                .l()?;
            let list = JObjectArray::from(list);
            let mut strings = Vec::new();
            for i in 0..env.get_array_length(&list)? {
                let item = JString::from(env.get_object_array_element(&list, i)?);
                strings.push(String::from(env.get_string(&item)?));
            }
            // Ids and labels, alternating
            devices = strings
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[id, label]| engine::Device { id: id.clone(), label: label.clone() })
                .collect();
            Ok(())
        });
        devices
    }

    fn start(&self, id: Option<&str>) -> Result<(), String> {
        let mut started = false;
        with_java(|env, shim, context| {
            let id = match id {
                Some(id) => JObject::from(env.new_string(id)?),
                None => JObject::null(),
            };
            env.call_static_method(
                shim,
                "cameraStart",
                "(Landroid/content/Context;Ljava/lang/String;)V",
                &[JValue::Object(context), JValue::Object(&id)],
            )?;
            started = true;
            Ok(())
        });
        if started { Ok(()) } else { Err("the camera couldn't be reached".into()) }
    }

    fn stop(&self) {
        with_java(|env, shim, _| env.call_static_method(shim, "cameraStop", "()V", &[]).map(drop));
    }
}

/// `Shim.nativeCameraAccess`: the camera permission was answered.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeCameraAccess(_env: JNIEnv, _class: JClass, granted: jboolean) {
    engine::android_camera::access(granted != 0);
}

/// `Camera.nativeFrame`: a captured frame, on the camera's thread. Its buffers are only valid during this call.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments, reason = "a JNI entry point, flat as Java calls it")]
pub extern "system" fn Java_com_danendz_call_Camera_nativeFrame(
    env: JNIEnv,
    _class: JClass,
    y: JByteBuffer,
    u: JByteBuffer,
    v: JByteBuffer,
    y_stride: jint,
    uv_stride: jint,
    uv_pixel_stride: jint,
    width: jint,
    height: jint,
    rotation: jint,
) {
    let plane = |buffer: &JByteBuffer| -> Option<&[u8]> {
        let ptr = env.get_direct_buffer_address(buffer).ok()?;
        let len = env.get_direct_buffer_capacity(buffer).ok()?;
        // SAFETY: an Image plane's direct buffer, which Java keeps open until this call returns
        (!ptr.is_null()).then(|| unsafe { std::slice::from_raw_parts(ptr, len) })
    };
    let (Some(y), Some(u), Some(v)) = (plane(&y), plane(&u), plane(&v)) else { return };
    let size = |n: jint| usize::try_from(n).unwrap_or(0);
    engine::android_camera::frame(&engine::android_camera::Planes {
        y,
        u,
        v,
        y_stride: size(y_stride),
        uv_stride: size(uv_stride),
        uv_pixel_stride: size(uv_pixel_stride),
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        rotation: u32::try_from(rotation).unwrap_or(0),
    });
}
