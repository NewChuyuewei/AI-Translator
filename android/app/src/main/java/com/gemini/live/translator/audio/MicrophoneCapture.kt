package com.gemini.live.translator.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicReference

class MicrophoneCapture {
    private val activeRecord = AtomicReference<AudioRecord?>(null)
    private val worker = AtomicReference<Job?>(null)

    @SuppressLint("MissingPermission")
    fun start(scope: CoroutineScope, onFrame: (ByteArray) -> Unit, onError: (String) -> Unit) {
        check(worker.get() == null) { "capture already active" }
        val minimum = AudioRecord.getMinBufferSize(16_000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (minimum <= 0) {
            onError("设备不支持 16 kHz 单声道录音")
            return
        }
        val record = runCatching {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                16_000,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minimum, PCM_FRAME_BYTES * 4),
            )
        }.getOrElse {
            onError("无法初始化麦克风")
            return
        }
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            onError("麦克风初始化失败")
            return
        }
        activeRecord.set(record)
        val job = scope.launch(Dispatchers.IO) {
            val frame = ByteArray(PCM_FRAME_BYTES)
            val silenceGate = SilenceGate()
            try {
                record.startRecording()
                while (isActive && activeRecord.get() === record) {
                    var offset = 0
                    while (offset < frame.size && isActive) {
                        val count = record.read(frame, offset, frame.size - offset, AudioRecord.READ_BLOCKING)
                        if (count <= 0) throw IllegalStateException("audio read failed")
                        offset += count
                    }
                    if (offset == frame.size && silenceGate.shouldSend(frame)) onFrame(frame.copyOf())
                }
            } catch (_: Exception) {
                if (activeRecord.get() === record) onError("麦克风采集意外停止")
            } finally {
                frame.fill(0)
                activeRecord.compareAndSet(record, null)
                runCatching { record.stop() }
                record.release()
            }
        }
        if (!worker.compareAndSet(null, job)) {
            job.cancel()
            activeRecord.compareAndSet(record, null)
            record.release()
            error("capture already active")
        }
        job.invokeOnCompletion { worker.compareAndSet(job, null) }
    }

    fun stop() {
        activeRecord.getAndSet(null)?.let { runCatching { it.stop() } }
        worker.getAndSet(null)?.cancel()
    }
}
