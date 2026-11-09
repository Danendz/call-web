package com.danendz.call

import android.content.ActivityNotFoundException
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import java.io.IOException

/**
 * Pictures of the call, put where the phone's gallery finds them. Android 10+ won't let an app write to the shared
 * Pictures folder by path, but MediaStore takes them without any storage permission.
 */
object Gallery {
    /** Saves a JPEG as Pictures/Call Danya/<name>.jpg; returns its content URI, or null if it couldn't. */
    fun save(context: Context, name: String, jpeg: ByteArray): String? {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "$name.jpg")
            put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/Call Danya")
            // Hidden from the gallery until it's all written
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val uri = try {
            resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
        } catch (e: Exception) {
            Log.w("call-danya", "screenshot not added to the gallery: $e")
            null
        } ?: return null
        return try {
            resolver.openOutputStream(uri)?.use { it.write(jpeg) } ?: throw IOException("no stream")
            resolver.update(uri, ContentValues().apply { put(MediaStore.Images.Media.IS_PENDING, 0) }, null, null)
            uri.toString()
        } catch (e: Exception) {
            Log.w("call-danya", "screenshot not written: $e")
            resolver.delete(uri, null, null)
            null
        }
    }

    /** Opens a saved picture in the phone's gallery or photo viewer. */
    fun open(context: Context, uri: String) {
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(Uri.parse(uri), "image/jpeg")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            Log.w("call-danya", "nothing on this phone opens pictures")
        }
    }
}
