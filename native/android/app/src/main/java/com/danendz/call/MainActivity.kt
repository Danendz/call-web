package com.danendz.call

import android.Manifest
import android.app.NativeActivity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle

/**
 * The app is the Rust library (`native/app`), drawn by Slint. This only hands WebRTC the app context from the main
 * thread, where its Java classes can be found, and asks for the mic before NativeActivity starts `android_main`.
 * The rest of what Android does for the app is in [Shim] and [CallService].
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        nativeInit(applicationContext)
        // The mic for the call, and on Android 13+ the call's notification
        val wanted = buildList {
            add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) {
            requestPermissions(wanted.toTypedArray(), 0)
        }
        super.onCreate(savedInstanceState)
    }

    // Back that no screen took (the system's, or Slint's fallback) would end the activity, and the call with it;
    // like a phone app, the call goes to the background instead. Swiping the app away from recents still quits it.
    override fun finish() {
        moveTaskToBack(true)
    }

    companion object {
        init {
            System.loadLibrary("call_danya")
        }

        @JvmStatic
        external fun nativeInit(context: Context)
    }
}
