package com.gemini.live.translator.audio

import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel

/** Forty milliseconds of 16 kHz mono PCM16. */
const val PCM_FRAME_BYTES: Int = 16_000 * 40 / 1_000 * 2
const val MAX_QUEUED_AUDIO_MS: Int = 240

class PcmFrameQueue {
    private val frames = Channel<ByteArray>(
        capacity = MAX_QUEUED_AUDIO_MS / 40,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )

    fun offer(frame: ByteArray): Boolean {
        require(frame.size == PCM_FRAME_BYTES) { "PCM frame must be exactly 40 ms" }
        // Copy at the ownership boundary so an AudioRecord buffer cannot be mutated later.
        return frames.trySend(frame.copyOf()).isSuccess
    }

    suspend fun receive(): ByteArray = frames.receive()
    fun close() = frames.close()
}
