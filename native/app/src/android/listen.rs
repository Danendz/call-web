//! Friends' calls with the app closed, behind Settings' "Get calls when the app is closed". `ListenService.kt` keeps
//! the process running, and this keeps a second `/ws/user` open in it, without the window, that always counts as in
//! the background. Its rings become Android's call notification (`Shim.kt`): Decline goes out on this socket, Accept
//! opens the app, whose own socket gets the ring when it connects and answers it ([`take_answer`]).

use std::sync::Mutex;

use engine::friends::{Account, Friends, FriendsCommand, FriendsState};
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean};

use super::with_java;

struct Listener {
    friends: Friends,
}

/// What the notifications were last told: the call ringing, and the missed calls already seen (None before the
/// first list, so old ones aren't announced)
#[derive(Default)]
struct Seen {
    ringing: Option<String>,
    missed: Option<Vec<(String, u64)>>,
}

static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);

/// `Shim.nativeListen(dir)`, from `ListenService` on the main thread: (re)connects with the saved session, read from
/// the app's files folder. False when nobody is signed in, and the service stops.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeListen(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
) -> jboolean {
    if let Ok(dir) = env.get_string(&dir) {
        let _ = crate::settings::ANDROID_DIR.set(String::from(dir).into());
    }
    let Ok(mut listener) = LISTENER.lock() else { return JNI_FALSE };
    // A new session (signed in again) replaces the old socket
    *listener = None;
    let Some(session) = crate::friends::load_session() else { return JNI_FALSE };
    let server = match crate::server_url() {
        Ok(server) => server,
        Err(e) => {
            tracing::warn!("listening: {e:#}");
            return JNI_FALSE;
        }
    };
    let seen = Mutex::new(Seen::default());
    let friends = Friends::start(server, Some(session), move |state| on_state(&state, &seen), |_| {});
    match friends {
        Ok(friends) => {
            friends.send(FriendsCommand::Foreground(false));
            *listener = Some(Listener { friends });
            tracing::info!("listening for calls");
            JNI_TRUE
        }
        Err(e) => {
            tracing::warn!("listening: {e}");
            JNI_FALSE
        }
    }
}

/// `Shim.nativeStopListening`, when the service is destroyed: closes the socket.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeStopListening(_env: JNIEnv, _class: JClass) {
    if let Ok(mut listener) = LISTENER.lock() {
        *listener = None;
    }
}

/// `Shim.nativeDecline(callId)`: Decline on the call notification.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_danendz_call_Shim_nativeDecline(mut env: JNIEnv, _class: JClass, call_id: JString) {
    let Ok(call_id) = env.get_string(&call_id).map(String::from) else { return };
    if let Ok(listener) = LISTENER.lock()
        && let Some(listener) = listener.as_ref()
    {
        listener.friends.send(FriendsCommand::Answer { call_id, accept: false });
    }
}

/// On the friends thread: rings and missed calls into notifications, and stopping once signed out.
fn on_state(state: &FriendsState, seen: &Mutex<Seen>) {
    let Ok(mut seen) = seen.lock() else { return };
    if let Account::SignedOut { .. } = state.account {
        with_java(|env, shim, context| {
            env.call_static_method(shim, "listenSignedOut", "(Landroid/content/Context;)V", &[JValue::Object(context)])
                .map(drop)
        });
        return;
    }

    let incoming = state.incoming.first();
    if seen.ringing.as_ref() != incoming.map(|c| &c.call_id) {
        if let Some(id) = seen.ringing.take() {
            call("ringEnded", &[&id]);
        }
        if let Some(call_now) = incoming {
            call("ringShow", &[&call_now.call_id, &call_now.from]);
            seen.ringing = Some(call_now.call_id.clone());
        }
    }

    if let Some(lists) = &state.lists {
        let now: Vec<(String, u64)> = lists.missed.iter().map(|m| (m.from.clone(), m.at)).collect();
        if let Some(known) = &seen.missed {
            for (from, _) in now.iter().filter(|m| !known.contains(m)) {
                call("missedShow", &[from]);
            }
        }
        seen.missed = Some(now);
    }
}

/// `Shim.<method>(context, strings…)`
fn call(method: &str, args: &[&str]) {
    with_java(|env, shim, context| {
        let mut values = vec![JValue::Object(context)];
        let strings = args.iter().map(|s| env.new_string(s)).collect::<Result<Vec<_>, _>>()?;
        values.extend(strings.iter().map(|s| JValue::Object(s)));
        let signature = format!("(Landroid/content/Context;{})V", "Ljava/lang/String;".repeat(args.len()));
        env.call_static_method(shim, method, signature, &values).map(drop)
    });
}

/// The call Accept was pressed for on its notification, once: the app answers it when its own socket shows it.
pub(super) fn take_answer() -> Option<String> {
    let mut answer = None;
    with_java(|env, shim, _| {
        let id = env.call_static_method(shim, "takeAnswer", "()Ljava/lang/String;", &[])?.l()?;
        if !id.is_null() {
            answer = Some(env.get_string(&JString::from(id))?.into());
        }
        Ok(())
    });
    answer
}

/// Turns the service on or off, as Settings' switch says, with the notifications' text in the app's language:
/// "calling you", missed call, accept, decline, and the service's own. Android keeps both for after a reboot.
pub(super) fn set(on: bool, labels: &[String]) {
    with_java(|env, shim, context| {
        let array = env.new_object_array(labels.len().try_into().unwrap_or(0), "java/lang/String", JObject::null())?;
        for (i, label) in labels.iter().enumerate() {
            let label = env.new_string(label)?;
            env.set_object_array_element(&array, i.try_into().unwrap_or(0), label)?;
        }
        env.call_static_method(
            shim,
            "setListening",
            "(Landroid/content/Context;Z[Ljava/lang/String;)V",
            &[JValue::Object(context), JValue::Bool(u8::from(on)), JValue::Object(&array)],
        )
        .map(drop)
    });
}
