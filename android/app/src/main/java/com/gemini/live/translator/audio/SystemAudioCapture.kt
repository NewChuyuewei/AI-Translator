package com.gemini.live.translator.audio

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class SystemAudioCapture {
    private val active = AtomicBoolean(false)
    private val activeRecord = AtomicReference<AudioRecord?>(null)
    private val worker = AtomicReference<Job?>(null)

    @SuppressLint("MissingPermission")
    fun start(
        scope: CoroutineScope,
        projection: MediaProjection,
        onFrame: (ByteArray) -> Unit,
        onError: (String) -> Unit,
    ) {
        check(worker.get() == null) { "system capture already active" }
        val minimum = AudioRecord.getMinBufferSize(
            PCM_SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minimum <= 0) {
            onError("设备不支持手机内声音采集")
            return
        }
        val callback = object : MediaProjection.Callback() {
            override fun onStop() {
                if (active.getAndSet(false)) onError("手机内声音授权已被系统收回")
            }
        }
        projection.registerCallback(callback, Handler(Looper.getMainLooper()))
        val captureConfig = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(PCM_SAMPLE_RATE)
            .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
            .build()
        val record = runCatching {
            AudioRecord.Builder()
                .setAudioFormat(format)
                .setBufferSizeInBytes(maxOf(minimum, PCM_FRAME_BYTES * 4))
                .setAudioPlaybackCaptureConfig(captureConfig)
                .build()
        }.getOrElse {
            projection.unregisterCallback(callback)
            onError("无法初始化手机内声音采集")
            return
        }
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            projection.unregisterCallback(callback)
            onError("手机内声音采集初始化失败")
            return
        }
        active.set(true)
        activeRecord.set(record)
        val job = scope.launch(Dispatchers.IO) {
            val frame = ByteArray(PCM_FRAME_BYTES)
            val silenceGate = SilenceGate()
            try {
                record.startRecording()
                while (isActive && active.get() && activeRecord.get() === record) {
                    var offset = 0
                    while (offset < frame.size && isActive) {
                        val count = record.read(frame, offset, frame.size - offset, AudioRecord.READ_BLOCKING)
                        if (count <= 0) error("system audio read failed")
                        offset += count
                    }
                    if (offset == frame.size && silenceGate.shouldSend(frame)) onFrame(frame.copyOf())
                }
            } catch (_: Exception) {
                if (active.getAndSet(false)) onError("手机内声音采集意外停止")
            } finally {
                frame.fill(0)
                activeRecord.compareAndSet(record, null)
                runCatching { record.stop() }
                record.release()
                projection.unregisterCallback(callback)
            }
        }
        if (!worker.compareAndSet(null, job)) {
            active.set(false)
            job.cancel()
            activeRecord.compareAndSet(record, null)
            record.release()
            projection.unregisterCallback(callback)
            error("system capture already active")
        }
        job.invokeOnCompletion { worker.compareAndSet(job, null) }
    }

    fun stop() {
        active.set(false)
        activeRecord.getAndSet(null)?.let { runCatching { it.stop() } }
        worker.getAndSet(null)?.cancel()
    }
}
