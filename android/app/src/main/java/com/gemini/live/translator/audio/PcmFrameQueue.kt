package com.gemini.live.translator.audio

import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel

/** Forty milliseconds of 16 kHz mono PCM16. */
const val PCM_SAMPLE_RATE: Int = 16_000
const val PCM_FRAME_BYTES: Int = PCM_SAMPLE_RATE * 40 / 1_000 * 2
const val MAX_QUEUED_AUDIO_MS: Int = 240

class SilenceGate(
    private val threshold: Double = 0.003,
    private val hangoverFrames: Int = 25,
) {
    private var quietFrames = 0

    fun shouldSend(frame: ByteArray): Boolean {
        require(frame.size == PCM_FRAME_BYTES)
        var sumSquares = 0.0
        var offset = 0
        while (offset < frame.size) {
            val sample = ((frame[offset].toInt() and 0xff) or (frame[offset + 1].toInt() shl 8)).toShort()
            val normalized = sample.toDouble() / 32_768.0
            sumSquares += normalized * normalized
            offset += 2
        }
        val rms = kotlin.math.sqrt(sumSquares / (frame.size / 2))
        return if (rms >= threshold) {
            quietFrames = 0
            true
        } else {
            quietFrames++
            quietFrames <= hangoverFrames
        }
    }
}

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
