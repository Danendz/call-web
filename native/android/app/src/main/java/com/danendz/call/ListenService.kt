package com.danendz.call

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.IBinder

/**
 * Settings' "Get calls when the app is closed": keeps the process running with a quiet notification, so the Rust
 * side's second `/ws/user` (`app/src/android/listen.rs`) stays open while the app is closed. No push service works
 * in China without a vendor SDK, so this is how a ring reaches a phone whose app isn't open. Its rings show as a
 * full-screen call notification with Accept and Decline, and missed ones as a notification of their own.
 */
class ListenService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        // Started after a reboot, nothing of the app has run yet
        MainActivity.nativeInit(applicationContext)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_DECLINE) {
            val callId = intent.getStringExtra(EXTRA_CALL_ID) ?: return START_STICKY
            Shim.nativeDecline(callId)
            ringEnded(this, callId)
            return START_STICKY
        }
        val labels = labels(this)
        val manager = getSystemService(NotificationManager::class.java)
        // Min: no sound, no icon in the status bar; it's only there because Android requires it
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_LISTEN, labels[LABEL_LISTENING], NotificationManager.IMPORTANCE_MIN),
        )
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, CHANNEL_LISTEN)
            .setSmallIcon(R.drawable.ic_call)
            .setContentTitle(labels[LABEL_LISTENING])
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(ID, notification)
        }
        if (!Shim.nativeListen(filesDir.path)) {
            // Nobody signed in: nothing to listen for until someone is
            stopSelf()
            return START_NOT_STICKY
        }
        // Killed for memory, Android starts it again
        return START_STICKY
    }

    override fun onDestroy() {
        Shim.nativeStopListening()
        super.onDestroy()
    }

    companion object {
        private const val ID = 2
        private const val RING_ID = 3
        private const val CHANNEL_LISTEN = "listen"
        private const val CHANNEL_RING = "ring"
        private const val CHANNEL_MISSED = "missed"
        private const val ACTION_DECLINE = "com.danendz.call.DECLINE"
        private const val EXTRA_CALL_ID = "call_id"
        const val EXTRA_ANSWER = "answer"
        private const val PREFS = "listen"
        private const val PREF_ON = "on"
        private const val PREF_LABELS = "labels"
        // A ring left behind (the process died mid-ring) goes away by itself; the server's ring is 45 s
        private const val RING_TIMEOUT_MS = 60_000L

        // Indexes into the labels Rust hands over: "Calling you", "Missed call", "Accept", "Decline", the listening
        // notification's title
        private const val LABEL_CALLING = 0
        private const val LABEL_MISSED = 1
        private const val LABEL_ACCEPT = 2
        private const val LABEL_DECLINE = 3
        private const val LABEL_LISTENING = 4
        private val DEFAULT_LABELS = arrayOf("Calling you", "Missed call", "Accept", "Decline", "Ready for friends' calls")
        private const val SEPARATOR = "\u001f"

        // Which ring the notification shows, so a late end for an older one leaves it
        private var ringing: String? = null

        /** Settings' switch: remembered for after a reboot, with the labels in the app's language. */
        fun set(context: Context, on: Boolean, labels: Array<String>) {
            val app = context.applicationContext
            app.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putBoolean(PREF_ON, on)
                .putString(PREF_LABELS, labels.joinToString(SEPARATOR))
                .apply()
            val intent = Intent(app, ListenService::class.java)
            app.stopService(intent)
            if (on) app.startForegroundService(intent)
        }

        fun startIfOn(context: Context) {
            val app = context.applicationContext
            if (app.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(PREF_ON, false)) {
                app.startForegroundService(Intent(app, ListenService::class.java))
            }
        }

        fun stop(context: Context) {
            context.applicationContext.stopService(Intent(context.applicationContext, ListenService::class.java))
        }

        private fun labels(context: Context): Array<String> {
            val saved = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(PREF_LABELS, null)
                ?.split(SEPARATOR)
            return if (saved != null && saved.size == DEFAULT_LABELS.size) saved.toTypedArray() else DEFAULT_LABELS
        }

        private fun canNotify(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

        /** A friend's call: rings with the phone's ringtone until answered, declined or ended. */
        fun ringShow(context: Context, callId: String, from: String) {
            // The app on screen shows the call itself
            if (Shim.shown || !canNotify(context)) return
            val labels = labels(context)
            val manager = context.getSystemService(NotificationManager::class.java)
            val channel = NotificationChannel(CHANNEL_RING, labels[LABEL_CALLING], NotificationManager.IMPORTANCE_HIGH)
            channel.setSound(
                RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build(),
            )
            manager.createNotificationChannel(channel)
            val open = PendingIntent.getActivity(
                context, 10, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
            )
            val accept = PendingIntent.getActivity(
                context, 11,
                Intent(context, MainActivity::class.java).putExtra(EXTRA_ANSWER, callId),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val decline = PendingIntent.getService(
                context, 12,
                Intent(context, ListenService::class.java).setAction(ACTION_DECLINE).putExtra(EXTRA_CALL_ID, callId),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val icon = Icon.createWithResource(context, R.drawable.ic_call)
            val notification = Notification.Builder(context, CHANNEL_RING)
                .setSmallIcon(R.drawable.ic_call)
                .setContentTitle(from)
                .setContentText(labels[LABEL_CALLING])
                .setCategory(Notification.CATEGORY_CALL)
                .setContentIntent(open)
                // Over the lock screen like a phone call, where the phone allows it
                .setFullScreenIntent(open, true)
                .setOngoing(true)
                .setTimeoutAfter(RING_TIMEOUT_MS)
                .addAction(Notification.Action.Builder(icon, labels[LABEL_DECLINE], decline).build())
                .addAction(Notification.Action.Builder(icon, labels[LABEL_ACCEPT], accept).build())
                .build()
            // Rings on until it's taken away
            notification.flags = notification.flags or Notification.FLAG_INSISTENT
            ringing = callId
            manager.notify(RING_ID, notification)
        }

        fun ringEnded(context: Context, callId: String) {
            if (ringing != callId) return
            ringing = null
            context.getSystemService(NotificationManager::class.java).cancel(RING_ID)
        }

        fun missedShow(context: Context, from: String) {
            if (Shim.shown || !canNotify(context)) return
            val labels = labels(context)
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_MISSED, labels[LABEL_MISSED], NotificationManager.IMPORTANCE_DEFAULT),
            )
            val open = PendingIntent.getActivity(
                context, 13, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = Notification.Builder(context, CHANNEL_MISSED)
                .setSmallIcon(R.drawable.ic_call)
                .setContentTitle(from)
                .setContentText(labels[LABEL_MISSED])
                .setCategory(Notification.CATEGORY_MISSED_CALL)
                .setContentIntent(open)
                .setAutoCancel(true)
                .build()
            // One per friend: a newer missed call replaces the last
            manager.notify("missed-$from", 0, notification)
        }
    }
}

/** Starts listening again after a reboot or an app update, if Settings' switch is on. */
class ListenStarter : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            ListenService.startIfOn(context)
        }
    }
}
