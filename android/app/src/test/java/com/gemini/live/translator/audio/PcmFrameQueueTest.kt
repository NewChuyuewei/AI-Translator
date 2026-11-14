package com.gemini.live.translator.audio

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class PcmFrameQueueTest {
    @Test
    fun keepsOnlyLatestBoundedAudio() = runTest {
        val queue = PcmFrameQueue()
        repeat(10) { index -> queue.offer(ByteArray(PCM_FRAME_BYTES) { index.toByte() }) }
        // Capacity is six frames (240 ms), so the first retained frame is index four.
        assertArrayEquals(ByteArray(PCM_FRAME_BYTES) { 4 }, queue.receive())
        assertEquals(240, MAX_QUEUED_AUDIO_MS)
        queue.close()
    }

    @Test
    fun silenceGateKeepsVadHangoverThenSuppressesSilence() {
        val gate = SilenceGate()
        val silence = ByteArray(PCM_FRAME_BYTES)
        repeat(25) { assertEquals(true, gate.shouldSend(silence)) }
        assertEquals(false, gate.shouldSend(silence))
        val speech = ByteArray(PCM_FRAME_BYTES).apply {
            for (index in indices step 2) {
                this[index] = 0x00
                this[index + 1] = 0x20
            }
        }
        assertEquals(true, gate.shouldSend(speech))
        assertEquals(true, gate.shouldSend(silence))
    }

    @Test
    fun rejectsNonFortyMillisecondFrames() {
        val queue = PcmFrameQueue()
        assertThrows(IllegalArgumentException::class.java) { queue.offer(ByteArray(10)) }
        queue.close()
    }
}
