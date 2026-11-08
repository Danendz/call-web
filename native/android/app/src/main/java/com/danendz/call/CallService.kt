package com.danendz.call

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.os.Build
import android.os.IBinder

/**
 * Keeps the call running while the app is in the background, which Android only allows a foreground service with a
 * notification. The notification mutes and hangs up; tapping it brings the call back.
 */
class CallService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_TOGGLE_MIC -> Shim.toggleMic()
            ACTION_HANG_UP -> Shim.hangUp()
            else -> {
                val notification = build(intent ?: return START_NOT_STICKY)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    // The camera keeps sending too, where it's allowed; claiming it without the permission throws
                    var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                    if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                        types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
                    }
                    startForeground(ID, notification, types)
                } else {
                    startForeground(ID, notification)
                }
            }
        }
        // Gone with the process: a call can't come back on its own
        return START_NOT_STICKY
    }

    private fun build(intent: Intent): Notification {
        val labels = intent.getStringArrayExtra(EXTRA_LABELS) ?: Array(LABELS) { "" }
        val micOn = intent.getBooleanExtra(EXTRA_MIC_ON, true)
        val manager = getSystemService(NotificationManager::class.java)
        // Low: no sound, it's there to tap
        manager.createNotificationChannel(NotificationChannel(CHANNEL, labels[0], NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_call)
            .setContentTitle(labels[0])
            .setContentText(labels[1])
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_CALL)
            .addAction(action(ACTION_TOGGLE_MIC, if (micOn) labels[2] else labels[3], 1))
            .addAction(action(ACTION_HANG_UP, labels[4], 2))
            .build()
    }

    private fun action(name: String, label: String, code: Int): Notification.Action {
        val intent = PendingIntent.getService(
            this, code, Intent(this, CallService::class.java).setAction(name), PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_call), label, intent).build()
    }

    companion object {
        private const val ID = 1
        private const val CHANNEL = "call"
        private const val ACTION_TOGGLE_MIC = "com.danendz.call.TOGGLE_MIC"
        private const val ACTION_HANG_UP = "com.danendz.call.HANG_UP"
        private const val EXTRA_MIC_ON = "mic_on"
        private const val EXTRA_LABELS = "labels"

        /** Title, text, "Mute", "Unmute", "Hang up", in the app's language. */
        private const val LABELS = 5

        fun show(context: Context, micOn: Boolean, labels: Array<String>) {
            val intent = Intent(context, CallService::class.java)
                .putExtra(EXTRA_MIC_ON, micOn)
                .putExtra(EXTRA_LABELS, labels)
            context.startForegroundService(intent)
        }

        fun hide(context: Context) {
            context.stopService(Intent(context, CallService::class.java))
        }
    }
}
