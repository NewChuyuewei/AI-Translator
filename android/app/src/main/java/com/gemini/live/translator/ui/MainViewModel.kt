package com.gemini.live.translator.ui

import android.content.Intent
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.gemini.live.translator.SessionRuntime
import com.gemini.live.translator.data.AudioSourceMode
import com.gemini.live.translator.data.CaptionState
import com.gemini.live.translator.data.GeminiMode
import com.gemini.live.translator.data.SessionConfig
import com.gemini.live.translator.data.SessionState
import com.gemini.live.translator.security.SecretStore
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class MainUiState(
    val apiKey: String = "",
    val targetLanguage: String = "zh-Hans",
    val mode: GeminiMode = GeminiMode.LIVE_TRANSLATE,
    val audioSource: AudioSourceMode = AudioSourceMode.MICROPHONE,
    val rememberKey: Boolean = false,
    val session: SessionState = SessionState.Idle,
    val caption: CaptionState = CaptionState(),
)

class MainViewModel(
    private val runtime: SessionRuntime,
    private val secrets: SecretStore,
) : ViewModel() {
    private val initialKey = secrets.loadApiKey()
    private val mutableState = MutableStateFlow(
        MainUiState(
            apiKey = initialKey.orEmpty(),
            rememberKey = initialKey != null,
            session = runtime.session.value,
            caption = runtime.caption.value,
        ),
    )
    val state: StateFlow<MainUiState> = mutableState.asStateFlow()

    init {
        viewModelScope.launch {
            runtime.session.collect { session -> mutableState.update { it.copy(session = session) } }
        }
        viewModelScope.launch {
            runtime.caption.collect { caption -> mutableState.update { it.copy(caption = caption) } }
        }
    }

    fun setApiKey(value: String) = mutableState.update { it.copy(apiKey = value.take(512)) }
    fun setTargetLanguage(value: String) = mutableState.update { it.copy(targetLanguage = value.take(24)) }
    fun setMode(value: GeminiMode) = mutableState.update { it.copy(mode = value, caption = CaptionState()) }
    fun setAudioSource(value: AudioSourceMode) = mutableState.update { it.copy(audioSource = value) }
    fun setRememberKey(value: Boolean) = mutableState.update { it.copy(rememberKey = value) }

    fun start(projectionData: Intent? = null) {
        val snapshot = mutableState.value
        if (snapshot.session !is SessionState.Idle && snapshot.session !is SessionState.Failed) return
        val config = runCatching { SessionConfig(snapshot.apiKey, snapshot.mode, snapshot.targetLanguage, snapshot.audioSource).validated() }
            .getOrElse { error ->
                mutableState.update { it.copy(session = SessionState.Failed(error.message ?: "配置无效")) }
                return
            }
        if (snapshot.rememberKey) secrets.saveApiKey(config.apiKey) else secrets.clear()
        runCatching { runtime.start(config, projectionData) }.onFailure {
            mutableState.update { state -> state.copy(session = SessionState.Failed("无法启动会话")) }
        }
    }

    fun stop() = runtime.stop()

    fun permissionDenied() {
        mutableState.update { it.copy(session = SessionState.Failed("需要录音和通知权限才能开始")) }
    }

    fun projectionPermissionDenied() {
        mutableState.update { it.copy(session = SessionState.Failed("需要手机内声音捕获授权")) }
    }
}
