package com.danendz.call

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.util.Range
import android.util.Size
import android.view.Display
import java.nio.ByteBuffer
import kotlin.math.abs

/**
 * The camera through Camera2. Frames come as YUV_420_888 on the camera's own thread and go straight to Rust, which
 * crops and turns them upright for WebRTC and the preview. Everything here runs on that thread, in order, so a stop
 * is done before the next start opens a camera.
 */
object Camera {
    private const val WIDTH = 1280
    private const val HEIGHT = 720
    private const val FPS = 30

    private val thread = HandlerThread("camera").apply { start() }
    private val handler = Handler(thread.looper)

    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    // Bumped by every start and stop, so a camera that opens after it was stopped closes again
    private var generation = 0

    /** Ids and labels, alternating; a label is @front, @back or @external, named by the app. */
    fun list(context: Context): Array<String> {
        val manager = context.getSystemService(CameraManager::class.java)
        return ordered(manager).flatMap { id ->
            val facing = when (manager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING)) {
                CameraCharacteristics.LENS_FACING_FRONT -> "@front"
                CameraCharacteristics.LENS_FACING_BACK -> "@back"
                else -> "@external"
            }
            listOf(id, facing)
        }.toTypedArray()
    }

    // Front cameras first: it's a call, and the first one is what runs when none was picked
    private fun ordered(manager: CameraManager): List<String> = manager.cameraIdList.sortedBy {
        manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) !=
            CameraCharacteristics.LENS_FACING_FRONT
    }

    /** The permission was checked by Shim before the engine starts a camera. */
    @SuppressLint("MissingPermission")
    fun start(context: Context, wanted: String?) {
        val app = context.applicationContext
        handler.post {
            close()
            val manager = app.getSystemService(CameraManager::class.java)
            val ids = ordered(manager)
            val id = wanted?.takeIf { it in ids } ?: ids.firstOrNull() ?: return@post
            val traits = manager.getCameraCharacteristics(id)
            val size = pickSize(traits)
            val sensor = traits.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
            val front = traits.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
            val display = app.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
            val images = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
            images.setOnImageAvailableListener({ r ->
                val image = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                image.use {
                    val (y, u, v) = it.planes
                    nativeFrame(
                        y.buffer, u.buffer, v.buffer, y.rowStride, u.rowStride, u.pixelStride,
                        it.width, it.height, rotation(sensor, front, display?.rotation ?: 0),
                    )
                }
            }, handler)
            reader = images
            val mine = ++generation
            manager.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    if (mine != generation) return camera.close()
                    device = camera
                    capture(camera, images, traits)
                }

                override fun onDisconnected(camera: CameraDevice) {
                    camera.close()
                    if (device == camera) device = null
                }

                override fun onError(camera: CameraDevice, error: Int) {
                    android.util.Log.w("call-danya", "camera error $error")
                    onDisconnected(camera)
                }
            }, handler)
        }
    }

    fun stop() {
        handler.post {
            generation++
            close()
        }
    }

    private fun close() {
        session?.close()
        session = null
        device?.close()
        device = null
        reader?.close()
        reader = null
    }

    @Suppress("DEPRECATION")
    private fun capture(camera: CameraDevice, images: ImageReader, traits: CameraCharacteristics) {
        val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
            addTarget(images.surface)
            pickFps(traits)?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
        }.build()
        // The list form: the SessionConfiguration one needs API 28 and an executor for no gain here
        camera.createCaptureSession(listOf(images.surface), object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(configured: CameraCaptureSession) {
                if (device != camera) return configured.close()
                session = configured
                configured.setRepeatingRequest(request, null, handler)
            }

            override fun onConfigureFailed(failed: CameraCaptureSession) {
                android.util.Log.w("call-danya", "camera session failed")
            }
        }, handler)
    }

    // 720p as on desktop, else the 16:9 size nearest to it, else the nearest at all
    private fun pickSize(traits: CameraCharacteristics): Size {
        val sizes = traits.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?.getOutputSizes(ImageFormat.YUV_420_888).orEmpty()
        return sizes.minByOrNull {
            val wide = it.width * 9 == it.height * 16
            abs(it.width * it.height - WIDTH * HEIGHT).toLong() + if (wide) 0 else Int.MAX_VALUE.toLong()
        } ?: Size(WIDTH, HEIGHT)
    }

    // Up to 30 fps, as steady as the camera allows: the range ending at 30 with the highest floor
    private fun pickFps(traits: CameraCharacteristics): Range<Int>? =
        traits.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.filter { it.upper == FPS }?.maxByOrNull { it.lower }

    // How far the sensor's picture turns clockwise to stand upright on the screen, as WebRTC's own capturer does
    private fun rotation(sensor: Int, front: Boolean, surface: Int): Int {
        val device = surface * 90
        return if (front) (sensor + device) % 360 else (sensor - device + 360) % 360
    }

    @JvmStatic
    private external fun nativeFrame(
        y: ByteBuffer, u: ByteBuffer, v: ByteBuffer, yStride: Int, uvStride: Int, uvPixelStride: Int,
        width: Int, height: Int, rotation: Int,
    )
}
