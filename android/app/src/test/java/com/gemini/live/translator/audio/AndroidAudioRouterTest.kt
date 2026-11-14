package com.gemini.live.translator.audio

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidAudioRouterTest {
    @Test
    fun mixerAveragesTwoPcm16SourcesWithoutOverflow() {
        val microphone = constantFrame(30_000)
        val device = constantFrame(20_000)
        val mixed = mixPcm16(microphone, device)
        assertEquals(25_000, firstSample(mixed))
    }

    @Test
    fun mixerPreservesSingleAvailableSource() {
        val microphone = constantFrame(-12_345)
        assertArrayEquals(microphone, mixPcm16(microphone, null))
    }

    private fun constantFrame(sample: Int): ByteArray = ByteArray(PCM_FRAME_BYTES).also { frame ->
        for (offset in frame.indices step 2) {
            frame[offset] = sample.toByte()
            frame[offset + 1] = (sample shr 8).toByte()
        }
    }

    private fun firstSample(frame: ByteArray): Int =
        ((frame[0].toInt() and 0xff) or (frame[1].toInt() shl 8)).toShort().toInt()
}
