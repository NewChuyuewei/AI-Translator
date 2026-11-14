package com.gemini.live.translator.data

enum class GeminiMode(val wireModel: String, val transcriptionOnly: Boolean, val displayName: String) {
    GENERAL_LIVE("models/gemini-3.8-live", false, "Gemini 3.8 Live"),
    EXTENDED_THINKING("models/gemini-3.8-live-extended-thinking", false, "Gemini 3.8 Extended Thinking"),
    LIVE_TRANSLATE("models/gemini-3.5-live-translate-preview", false, "Gemini 3.5 实时翻译"),
    LIVE_TRANSCRIBE("models/gemini-3.5-transcribe-live", true, "Gemini 3.5 实时转写"),
}

enum class AudioSourceMode(val usesMicrophone: Boolean, val usesDeviceAudio: Boolean, val displayName: String) {
    MICROPHONE(true, false, "仅麦克风"),
    DEVICE_AUDIO(false, true, "仅手机内声音"),
    MIXED(true, true, "麦克风 + 手机内声音"),
}

data class SessionConfig(
    val apiKey: String,
    val mode: GeminiMode = GeminiMode.LIVE_TRANSLATE,
    val targetLanguage: String = "zh-Hans",
    val audioSource: AudioSourceMode = AudioSourceMode.MICROPHONE,
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
