package com.danendz.call

import android.Manifest
import android.app.NativeActivity
import android.content.Context
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import java.lang.ref.WeakReference

/**
 * The app is the Rust library (`native/app`), drawn by Slint. This only hands WebRTC the app context from the main
 * thread, where its Java classes can be found, and asks for the mic and camera before NativeActivity starts `android_main`.
 * The rest of what Android does for the app is in [Shim] and [CallService].
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        nativeInit(applicationContext)
        Shim.activity = WeakReference(this)
        // The mic and camera for the call, and on Android 13+ the call's notification; asked together, since a
        // second prompt can't come up while one is showing
        val wanted = buildList {
            add(Manifest.permission.RECORD_AUDIO)
            add(Manifest.permission.CAMERA)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) {
            Shim.asking = true
            requestPermissions(wanted.toTypedArray(), 0)
        }
        super.onCreate(savedInstanceState)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        Shim.permissionsAnswered(this, permissions)
    }

    // Android 11 and older: leaving during a call shrinks it into a picture-in-picture window (12+ does it by itself)
    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Shim.inCall && Build.VERSION.SDK_INT < Build.VERSION_CODES.S) pip()
    }

    private fun pip(): Boolean = try {
        enterPictureInPictureMode(Shim.pipParams(this))
    } catch (_: IllegalStateException) {
        // No picture-in-picture on this device
        false
    }

    override fun onPictureInPictureModeChanged(pip: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(pip, newConfig)
        Shim.nativePip(pip)
    }

    // Back that no screen took (the system's, or Slint's fallback) would end the activity, and the call with it;
    // like a phone app, the call goes to the background instead, in picture-in-picture during a call. Swiping the app
    // away from recents still quits it.
    override fun finish() {
        if (Shim.inCall && pip()) return
        moveTaskToBack(true)
    }

    // Android 12+ moves a root activity back on Back without finishing it
    @Deprecated("Still called while predictive back isn't opted into")
    override fun onBackPressed() {
        if (Shim.inCall && pip()) return
        @Suppress("DEPRECATION")
        super.onBackPressed()
    }

    companion object {
        init {
            System.loadLibrary("call_danya")
        }

        @JvmStatic
        external fun nativeInit(context: Context)
    }
}
