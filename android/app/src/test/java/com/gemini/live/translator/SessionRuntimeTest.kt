package com.gemini.live.translator

import com.gemini.live.translator.data.CaptionState
import org.junit.Assert.assertEquals
import org.junit.Test

class SessionRuntimeTest {
    @Test
    fun partialCaptionEventsDoNotEraseTheOtherLanguage() {
        val original = mergeCaption(CaptionState(), CaptionState(original = "hello"))
        val translated = mergeCaption(original, CaptionState(translated = "你好"))
        assertEquals("hello", translated.original)
        assertEquals("你好", translated.translated)

        val nextOriginal = mergeCaption(translated, CaptionState(original = "next sentence"))
        assertEquals("next sentence", nextOriginal.original)
        assertEquals("你好", nextOriginal.translated)
    }
}
