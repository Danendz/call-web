package com.danendz.call

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.provider.Settings
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates from our server: `{feed}/android.json` names the newest APK, its version code and SHA-256
 * (packaging/android/release.sh). A newer one downloads quietly into the app's cache and, once its hash matches,
 * shows the start screen's update button; the button hands it to the system installer, which asks to confirm.
 * Android itself refuses an APK not signed with the installed app's key.
 */
object Updater {
    private const val CHECK_EVERY_MS = 4 * 60 * 60 * 1000L
    // Far above the APK's size, so a broken feed can't fill the phone
    private const val MAX_BYTES = 300L * 1024 * 1024
    const val ACTION_INSTALLED = "com.danendz.call.UPDATE_INSTALLED"
    // A file at the feed's root, as release.sh names them
    private val FILE = Regex("""^call-danya-\d+\.\d+\.\d+\.apk$""")
    private val SHA256 = Regex("^[0-9a-f]{64}$")

    // Network and file work runs here, in order
    private val thread = HandlerThread("updates").apply { start() }
    private val handler = Handler(thread.looper)
    private var started = false
    // The verified APK, once one is downloaded
    private var ready: File? = null

    /** Checks now and every few hours until an update is downloaded; Rust hears of it through [nativeUpdateReady]. */
    fun start(context: Context, feed: String) {
        val app = context.applicationContext
        handler.post {
            if (started) return@post
            started = true
            check(app, feed.trimEnd('/'))
        }
    }

    private fun check(app: Context, feed: String) {
        val found = try {
            fetch(app, feed)
        } catch (e: Exception) {
            Log.w("call-danya", "update check failed: $e")
            // A download cut off halfway shows no progress until the next try
            nativeUpdateProgress(-1)
            null
        }
        if (found == null) {
            handler.postDelayed({ check(app, feed) }, CHECK_EVERY_MS)
            return
        }
        ready = found.first
        // Nothing newer will be installed before this one, so checking stops
        nativeUpdateReady(found.second)
    }

    /** The newer APK, downloaded and verified, with its version; null when this one is the newest. */
    private fun fetch(app: Context, feed: String): Pair<File, String>? {
        val dir = File(app.cacheDir, "updates").apply { mkdirs() }
        val json = JSONObject(get("$feed/android.json") { it.inputStream.readBytes().decodeToString() })
        val version = json.getString("version")
        val code = json.getLong("code")
        val name = json.getString("file")
        val sha256 = json.getString("sha256")
        require(FILE.matches(name) && SHA256.matches(sha256)) { "bad feed" }
        val installed = app.packageManager.getPackageInfo(app.packageName, 0).longVersionCode
        // An older download, or this version's after it was installed
        dir.listFiles()?.filter { it.name != name }?.forEach { it.delete() }
        if (code <= installed) {
            File(dir, name).delete()
            return null
        }

        val apk = File(dir, name)
        if (apk.exists() && hash(apk) == sha256) return apk to version
        val part = File(dir, "$name.part")
        get("$feed/$name") { connection ->
            val size = connection.contentLengthLong
            if (size > MAX_BYTES) error("update too big")
            var shown = -1
            connection.inputStream.use { input ->
                part.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    var total = 0L
                    while (true) {
                        val n = input.read(buffer)
                        if (n < 0) break
                        total += n
                        if (total > MAX_BYTES) error("update too big")
                        output.write(buffer, 0, n)
                        val percent = if (size > 0) (total * 100 / size).toInt() else 0
                        if (percent != shown) {
                            shown = percent
                            nativeUpdateProgress(percent)
                        }
                    }
                }
            }
        }
        if (hash(part) != sha256) {
            part.delete()
            error("update's SHA-256 doesn't match")
        }
        if (!part.renameTo(apk)) error("couldn't keep the update")
        return apk to version
    }

    private fun <T> get(url: String, read: (HttpURLConnection) -> T): T {
        val connection = URL(url).openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 15_000
            connection.readTimeout = 30_000
            if (connection.responseCode != 200) error("$url answered ${connection.responseCode}")
            return read(connection)
        } finally {
            connection.disconnect()
        }
    }

    private fun hash(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                digest.update(buffer, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /** The start screen's update button: installs the downloaded APK, after the system's confirmation. */
    fun install(context: Context) {
        val app = context.applicationContext
        handler.post {
            val apk = ready ?: return@post
            // Android 8+ asks once per app whether it may install apps; its settings page is where that's allowed
            if (!app.packageManager.canRequestPackageInstalls()) {
                app.startActivity(
                    Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${app.packageName}"))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
                return@post
            }
            try {
                commit(app, apk)
            } catch (e: Exception) {
                Log.w("call-danya", "update install failed: $e")
            }
        }
    }

    private fun commit(app: Context, apk: File) {
        val installer = app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
            .apply { setAppPackageName(app.packageName) }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite("call-danya.apk", 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            // The installer's answer comes back to MainActivity; mutable, so the installer can add its status
            val answer = Intent(app, MainActivity::class.java).setAction(ACTION_INSTALLED)
            val pending = PendingIntent.getActivity(
                app, 0, answer, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
            )
            session.commit(pending.intentSender)
        }
    }

    /** The installer's answer, given to MainActivity. On success Android has already replaced and stopped the app. */
    fun answered(context: Context, intent: Intent) {
        if (intent.action != ACTION_INSTALLED) return
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_INTENT)
                }
                confirm?.let { context.startActivity(it) }
            }
            PackageInstaller.STATUS_SUCCESS -> {}
            else -> Log.w(
                "call-danya",
                "update not installed: $status ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}",
            )
        }
    }

    @JvmStatic
    private external fun nativeUpdateProgress(percent: Int)

    @JvmStatic
    private external fun nativeUpdateReady(version: String)
}
