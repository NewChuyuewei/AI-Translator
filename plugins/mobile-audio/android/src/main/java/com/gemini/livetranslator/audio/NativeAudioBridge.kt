package com.gemini.livetranslator.audio

object NativeAudioBridge {
    init { System.loadLibrary("gemini_live_translator_lib") }
    @JvmStatic external fun pushAudio(system: Boolean, data: ShortArray, length: Int, sampleRate: Int)
    @JvmStatic external fun reportError(message: String)
}
