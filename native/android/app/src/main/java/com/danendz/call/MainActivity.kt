package com.danendz.call

import android.Manifest
import android.app.NativeActivity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Bundle

/**
 * The app is the Rust library (`native/app`), drawn by Slint. This only hands WebRTC the app context from the main
 * thread, where its Java classes can be found, and asks for the mic before NativeActivity starts `android_main`.
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        nativeInit(applicationContext)
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 0)
        }
        super.onCreate(savedInstanceState)
    }

    companion object {
        init {
            System.loadLibrary("call_danya")
        }

        @JvmStatic
        external fun nativeInit(context: Context)
    }
}
