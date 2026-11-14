package com.gemini.live.translator.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class SessionConfigTest {
    @Test
    fun trimsAndValidatesSecretsWithoutEchoingThem() {
        val result = SessionConfig("  private-key  ", targetLanguage = "zh-Hans").validated()
        assertEquals("private-key", result.apiKey)
        assertThrows(IllegalArgumentException::class.java) { SessionConfig("", targetLanguage = "zh").validated() }
        assertThrows(IllegalArgumentException::class.java) { SessionConfig("key\nvalue", targetLanguage = "zh").validated() }
        assertThrows(IllegalArgumentException::class.java) { SessionConfig("key", targetLanguage = "../../etc").validated() }
    }
}
