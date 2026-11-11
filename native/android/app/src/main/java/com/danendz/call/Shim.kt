package com.danendz.call

import android.Manifest
import android.app.Activity
import android.app.PictureInPictureParams
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.util.Rational
import java.lang.ref.WeakReference

/**
 * What the Rust app asks of Android, and what Android tells it back. Rust calls these from its own thread; the work
 * moves to the main thread, where Android's audio callbacks also arrive.
 */
object Shim {
    // Must match app/src/android/mod.rs
    private const val ACTION_TOGGLE_MIC = 1
    private const val ACTION_HANG_UP = 2
    private const val ACTION_INTERRUPTED = 3
    private const val ACTION_RESUMED = 4

    private const val ROUTE_SPEAKER = 0
    private const val ROUTE_EARPIECE = 1
    private const val ROUTE_BLUETOOTH = 2
    // A wired or USB headset; its mic comes with it, as Bluetooth's does
    private const val ROUTE_HEADSET = 3

    private val main = Handler(Looper.getMainLooper())
    var inCall = false
        private set
    // The main video's shape, which the picture-in-picture window takes
    private var aspect = Rational(16, 9)
    private var route = ROUTE_SPEAKER
    // The person picked the route; a headset coming or going no longer moves it
    private var picked = false
    private var focus: AudioFocusRequest? = null
    private var proximity: PowerManager.WakeLock? = null
    // For permission prompts, which only an activity can show; set by MainActivity
    var activity = WeakReference<Activity>(null)
    // The activity's prompt is up; its answer goes to Rust when it comes
    var asking = false

    /** The call started or ended, or the mic or the notification's text changed. */
    @JvmStatic
    fun callState(
        context: Context, inCall: Boolean, micOn: Boolean, labels: Array<String>, width: Int, height: Int,
    ) {
        val app = context.applicationContext
        main.post {
            if (width > 0 && height > 0) aspect = Rational(width, height)
            if (inCall) {
                if (!this.inCall) startAudio(app)
                CallService.show(app, micOn, labels)
            } else if (this.inCall) {
                stopAudio(app)
                CallService.hide(app)
            }
            this.inCall = inCall
            activity.get()?.let { pipParams(it) }
        }
    }

    /** What picture-in-picture shows like; on Android 12+ it also comes up by itself on leaving during a call. */
    fun pipParams(activity: Activity): PictureInPictureParams {
        // Android refuses shapes beyond 2.39:1 either way
        val ratio = aspect.toFloat().coerceIn(1 / 2.39f, 2.39f)
        val shape = if (ratio == aspect.toFloat()) aspect else Rational((ratio * 1000).toInt(), 1000)
        val params = PictureInPictureParams.Builder().setAspectRatio(shape).apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) setAutoEnterEnabled(inCall)
        }.build()
        try {
            activity.setPictureInPictureParams(params)
        } catch (_: IllegalStateException) {
            // No picture-in-picture on this device
        }
        return params
    }

    @JvmStatic
    fun setRoute(context: Context, route: Int) {
        val app = context.applicationContext
        main.post {
            picked = true
            applyRoute(app, route)
        }
    }

    /** Lowercased, e.g. "huawei": which vendor's battery and app launch settings to explain. */
    @JvmStatic
    fun vendor(): String = Build.MANUFACTURER.lowercase()

    /** The vendor's app launch page where there is one (Huawei, Honor), else the app's own settings. */
    @JvmStatic
    fun openBackgroundSettings(context: Context) {
        val app = context.applicationContext
        val launch = Intent().setComponent(
            ComponentName(
                "com.huawei.systemmanager",
                "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity",
            ),
        )
        val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + app.packageName))
        main.post {
            for (intent in listOf(launch, details)) {
                try {
                    app.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    return@post
                } catch (_: Exception) {
                    // Not on this phone; try the next
                }
            }
        }
    }

    /** Asks for the camera unless it's allowed; the answer goes to [nativeCameraAccess]. */
    @JvmStatic
    fun requestCamera(context: Context) {
        val app = context.applicationContext
        main.post {
            if (app.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                nativeCameraAccess(true)
            } else if (!asking) {
                val shown = activity.get() ?: return@post nativeCameraAccess(false)
                asking = true
                shown.requestPermissions(arrayOf(Manifest.permission.CAMERA), 0)
            }
        }
    }

    /** MainActivity's prompt was answered. */
    fun permissionsAnswered(context: Context, asked: Array<out String>) {
        asking = false
        if (Manifest.permission.CAMERA in asked) {
            nativeCameraAccess(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
        }
    }

    @JvmStatic
    fun cameraList(context: Context): Array<String> = Camera.list(context)

    @JvmStatic
    fun cameraStart(context: Context, id: String?) = Camera.start(context, id)

    @JvmStatic
    fun cameraStop() = Camera.stop()

    @JvmStatic
    fun savePicture(context: Context, name: String, jpeg: ByteArray): String? = Gallery.save(context, name, jpeg)

    @JvmStatic
    fun openPicture(context: Context, uri: String) = Gallery.open(context, uri)

    @JvmStatic
    fun startUpdates(context: Context, feed: String) = Updater.start(context, feed)

    @JvmStatic
    fun installUpdate(context: Context) = Updater.install(context)

    @JvmStatic
    external fun nativeAction(action: Int)

    @JvmStatic
    external fun nativeCameraAccess(granted: Boolean)

    @JvmStatic
    external fun nativePip(pip: Boolean)

    @JvmStatic
    external fun nativeShown(shown: Boolean)

    /** Which routes there are, as bits of 1 << route, and the one in use. */
    @JvmStatic
    external fun nativeAudio(available: Int, route: Int)

    fun toggleMic() = nativeAction(ACTION_TOGGLE_MIC)

    fun hangUp() = nativeAction(ACTION_HANG_UP)

    private val devices = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = devicesChanged()

        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = devicesChanged()
    }

    private lateinit var audio: AudioManager
    private lateinit var appContext: Context

    private fun startAudio(app: Context) {
        appContext = app
        audio = app.getSystemService(AudioManager::class.java)
        // The phone's call path: its echo canceller, and the earpiece to route to
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            // A phone call takes the audio: the mic mutes until it's over
            .setOnAudioFocusChangeListener({ change ->
                when (change) {
                    AudioManager.AUDIOFOCUS_LOSS, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> nativeAction(ACTION_INTERRUPTED)
                    AudioManager.AUDIOFOCUS_GAIN -> nativeAction(ACTION_RESUMED)
                }
            }, main)
            .build()
        audio.requestAudioFocus(request)
        focus = request
        picked = false
        audio.registerAudioDeviceCallback(devices, main)
        devicesChanged()
    }

    private fun stopAudio(app: Context) {
        audio.unregisterAudioDeviceCallback(devices)
        focus?.let { audio.abandonAudioFocusRequest(it) }
        focus = null
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            audio.clearCommunicationDevice()
        } else {
            legacyRoute(ROUTE_EARPIECE)
        }
        audio.mode = AudioManager.MODE_NORMAL
        holdProximity(app, false)
    }

    // A tablet has no earpiece
    private fun available(): Int {
        val outputs = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
        var bits = 1 shl ROUTE_SPEAKER
        if (outputs.any { it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE }) bits = bits or (1 shl ROUTE_EARPIECE)
        if (outputs.any { isBluetooth(it.type) }) bits = bits or (1 shl ROUTE_BLUETOOTH)
        if (outputs.any { isHeadset(it.type) }) bits = bits or (1 shl ROUTE_HEADSET)
        return bits
    }

    private fun isBluetooth(type: Int) =
        type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ||
            (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && type == AudioDeviceInfo.TYPE_BLE_HEADSET)

    private fun isHeadset(type: Int) =
        type == AudioDeviceInfo.TYPE_WIRED_HEADSET || type == AudioDeviceInfo.TYPE_WIRED_HEADPHONES ||
            type == AudioDeviceInfo.TYPE_USB_HEADSET

    // A headset that comes along takes the call, as on a phone call, unless a route was picked; the speaker takes it
    // back when it goes
    private fun devicesChanged() {
        if (focus == null) return
        val bits = available()
        val next = when {
            picked && bits and (1 shl route) != 0 -> route
            bits and (1 shl ROUTE_BLUETOOTH) != 0 -> ROUTE_BLUETOOTH
            bits and (1 shl ROUTE_HEADSET) != 0 -> ROUTE_HEADSET
            else -> ROUTE_SPEAKER
        }
        applyRoute(appContext, next)
    }

    private fun applyRoute(app: Context, wanted: Int) {
        if (focus == null) return
        val bits = available()
        val next = if (bits and (1 shl wanted) != 0) wanted else ROUTE_SPEAKER
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val type = when (next) {
                ROUTE_EARPIECE -> AudioDeviceInfo.TYPE_BUILTIN_EARPIECE
                else -> AudioDeviceInfo.TYPE_BUILTIN_SPEAKER
            }
            val device = audio.availableCommunicationDevices.firstOrNull {
                when (next) {
                    ROUTE_BLUETOOTH -> isBluetooth(it.type)
                    ROUTE_HEADSET -> isHeadset(it.type)
                    else -> it.type == type
                }
            }
            if (device != null) audio.setCommunicationDevice(device)
        } else {
            legacyRoute(next)
        }
        route = next
        holdProximity(app, next == ROUTE_EARPIECE)
        nativeAudio(bits, next)
    }

    // Android 10 and 11 have no communication device to pick
    @Suppress("DEPRECATION")
    private fun legacyRoute(next: Int) {
        if (next == ROUTE_BLUETOOTH) {
            audio.isSpeakerphoneOn = false
            audio.startBluetoothSco()
            audio.isBluetoothScoOn = true
        } else {
            // Off the speaker, a plugged-in headset takes over from the earpiece by itself
            audio.stopBluetoothSco()
            audio.isBluetoothScoOn = false
            audio.isSpeakerphoneOn = next == ROUTE_SPEAKER
        }
    }

    // At the ear, the screen goes dark so a cheek doesn't press anything
    private fun holdProximity(app: Context, hold: Boolean) {
        if (hold && proximity == null) {
            val power = app.getSystemService(PowerManager::class.java)
            if (power.isWakeLockLevelSupported(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK)) {
                proximity = power.newWakeLock(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK, "call-danya:proximity")
                    .also { it.acquire() }
            }
        } else if (!hold) {
            proximity?.release()
            proximity = null
        }
    }
}
