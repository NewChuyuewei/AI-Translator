package com.gemini.live.translator.audio

import android.media.projection.MediaProjection
import com.gemini.live.translator.data.AudioSourceMode
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicReference

private enum class CaptureSource { MICROPHONE, DEVICE }
private data class CapturedFrame(val source: CaptureSource, val bytes: ByteArray)

class AndroidAudioRouter {
    private val microphone = MicrophoneCapture()
    private val systemAudio = SystemAudioCapture()
    private val mixerJob = AtomicReference<Job?>(null)
    private val projection = AtomicReference<MediaProjection?>(null)
    private val mixedFrames = Channel<CapturedFrame>(
        capacity = 12,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
        onUndeliveredElement = { it.bytes.fill(0) },
    )

    fun start(
        scope: CoroutineScope,
        mode: AudioSourceMode,
        mediaProjection: MediaProjection?,
        onFrame: (ByteArray) -> Unit,
        onError: (String) -> Unit,
    ) {
        check(mixerJob.get() == null) { "audio router already active" }
        if (mode.usesDeviceAudio) {
            requireNotNull(mediaProjection) { "手机内声音授权无效" }
            projection.set(mediaProjection)
        }
        val route: (CaptureSource, ByteArray) -> Unit = if (mode == AudioSourceMode.MIXED) {
            startMixer(scope, onFrame)
            { source, bytes ->
                if (!mixedFrames.trySend(CapturedFrame(source, bytes)).isSuccess) bytes.fill(0)
            }
        } else {
            { _, bytes -> onFrame(bytes) }
        }
        if (mode.usesMicrophone) {
            microphone.start(
                scope,
                onFrame = { route(CaptureSource.MICROPHONE, it) },
                onError = onError,
            )
        }
        if (mode.usesDeviceAudio) {
            systemAudio.start(
                scope,
                projection = requireNotNull(mediaProjection),
                onFrame = { route(CaptureSource.DEVICE, it) },
                onError = onError,
            )
        }
    }

    private fun startMixer(scope: CoroutineScope, onFrame: (ByteArray) -> Unit) {
        val job = scope.launch {
            while (isActive) {
                delay(40)
                var microphoneFrame: ByteArray? = null
                var deviceFrame: ByteArray? = null
                while (true) {
                    val captured = mixedFrames.tryReceive().getOrNull() ?: break
                    when (captured.source) {
                        CaptureSource.MICROPHONE -> {
                            microphoneFrame?.fill(0)
                            microphoneFrame = captured.bytes
                        }
                        CaptureSource.DEVICE -> {
                            deviceFrame?.fill(0)
                            deviceFrame = captured.bytes
                        }
                    }
                }
                if (microphoneFrame == null && deviceFrame == null) continue
                val output = mixPcm16(microphoneFrame, deviceFrame)
                microphoneFrame?.fill(0)
                deviceFrame?.fill(0)
                onFrame(output)
            }
        }
        check(mixerJob.compareAndSet(null, job)) { "audio mixer already active" }
        job.invokeOnCompletion { mixerJob.compareAndSet(job, null) }
    }

    fun stop() {
        microphone.stop()
        systemAudio.stop()
        mixerJob.getAndSet(null)?.cancel()
        while (true) mixedFrames.tryReceive().getOrNull()?.bytes?.fill(0) ?: break
        projection.getAndSet(null)?.stop()
    }
}

internal fun mixPcm16(microphone: ByteArray?, device: ByteArray?): ByteArray {
    require(microphone == null || microphone.size == PCM_FRAME_BYTES)
    require(device == null || device.size == PCM_FRAME_BYTES)
    val output = ByteArray(PCM_FRAME_BYTES)
    var offset = 0
    while (offset < output.size) {
        val mic = microphone?.let { ((it[offset].toInt() and 0xff) or (it[offset + 1].toInt() shl 8)).toShort().toInt() } ?: 0
        val system = device?.let { ((it[offset].toInt() and 0xff) or (it[offset + 1].toInt() shl 8)).toShort().toInt() } ?: 0
        val activeSources = (if (microphone != null) 1 else 0) + (if (device != null) 1 else 0)
        val mixed = ((mic + system) / activeSources.coerceAtLeast(1)).coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt())
        output[offset] = mixed.toByte()
        output[offset + 1] = (mixed shr 8).toByte()
        offset += 2
    }
    return output
}
