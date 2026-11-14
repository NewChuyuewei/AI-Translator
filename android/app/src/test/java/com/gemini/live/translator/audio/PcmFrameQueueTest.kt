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
    fun rejectsNonFortyMillisecondFrames() {
        val queue = PcmFrameQueue()
        assertThrows(IllegalArgumentException::class.java) { queue.offer(ByteArray(10)) }
        queue.close()
    }
}
