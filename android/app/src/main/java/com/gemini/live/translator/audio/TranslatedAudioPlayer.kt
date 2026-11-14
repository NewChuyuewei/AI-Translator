package com.gemini.live.translator.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicReference

class TranslatedAudioPlayer {
    private val track = AtomicReference<AudioTrack?>(null)
    private val worker = AtomicReference<Job?>(null)
    private val audio = Channel<ByteArray>(capacity = 12, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    fun start(scope: CoroutineScope, onError: (String) -> Unit) {
        if (worker.get() != null) return
        val minimum = AudioTrack.getMinBufferSize(24_000, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (minimum <= 0) {
            onError("设备不支持 24 kHz 译音播放")
            return
        }
        val player = runCatching {
            AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_ASSISTANCE_ACCESSIBILITY)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(24_000)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(maxOf(minimum, 24_000 * 2 * 12 / 25))
                .build()
        }.getOrElse {
            onError("无法初始化译音播放器")
            return
        }
        if (player.state != AudioTrack.STATE_INITIALIZED) {
            player.release()
            onError("译音播放器初始化失败")
            return
        }
        track.set(player)
        val job = scope.launch(Dispatchers.IO) {
            try {
                player.play()
                while (isActive) {
                    val bytes = audio.receive()
                    try {
                        var offset = 0
                        while (offset < bytes.size && isActive) {
                            val count = player.write(bytes, offset, bytes.size - offset, AudioTrack.WRITE_BLOCKING)
                            if (count <= 0) error("audio output failed")
                            offset += count
                        }
                    } finally {
                        bytes.fill(0)
                    }
                }
            } catch (_: Exception) {
                if (track.get() === player && isActive) onError("译音播放意外停止")
            } finally {
                track.compareAndSet(player, null)
                runCatching { player.stop() }
                player.flush()
                player.release()
            }
        }
        if (!worker.compareAndSet(null, job)) {
            job.cancel()
            track.compareAndSet(player, null)
            player.release()
        }
        job.invokeOnCompletion { worker.compareAndSet(job, null) }
    }

    fun offer(bytes: ByteArray): Boolean = audio.trySend(bytes.copyOf()).isSuccess

    fun stop() {
        track.getAndSet(null)?.let { runCatching { it.pause() }; it.flush() }
        worker.getAndSet(null)?.cancel()
        while (true) audio.tryReceive().getOrNull()?.fill(0) ?: break
    }
}
