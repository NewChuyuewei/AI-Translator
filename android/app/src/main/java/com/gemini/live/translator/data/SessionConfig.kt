package com.gemini.live.translator.data

enum class GeminiMode(val wireModel: String, val transcriptionOnly: Boolean) {
    LIVE_TRANSLATE("models/gemini-3.5-live-translate-preview", false),
    LIVE_TRANSCRIBE("models/gemini-3.5-transcribe-live", true),
}

data class SessionConfig(
    val apiKey: String,
    val mode: GeminiMode = GeminiMode.LIVE_TRANSLATE,
    val targetLanguage: String = "zh-Hans",
) {
    fun validated(): SessionConfig {
        val key = apiKey.trim()
        require(key.isNotEmpty() && key.length <= 512) { "API Key 长度无效" }
        require(key.none { it.isISOControl() }) { "API Key 含无效字符" }
        require(targetLanguage.length in 2..24 && targetLanguage.all { it.isLetter() || it == '-' }) {
            "目标语言代码无效"
        }
        return copy(apiKey = key)
    }
}

sealed interface SessionState {
    data object Idle : SessionState
    data object Connecting : SessionState
    data object Running : SessionState
    data object Stopping : SessionState
    data class Failed(val safeMessage: String) : SessionState
}

data class CaptionState(
    val original: String = "",
    val translated: String = "",
)
