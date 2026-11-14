package com.gemini.live.translator.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.gemini.live.translator.audio.MicrophoneCapture
import com.gemini.live.translator.data.CaptionState
import com.gemini.live.translator.data.GeminiMode
import com.gemini.live.translator.data.SessionConfig
import com.gemini.live.translator.data.SessionState
import com.gemini.live.translator.network.ClientEvent
import com.gemini.live.translator.network.SessionClient
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
    val rememberKey: Boolean = false,
    val session: SessionState = SessionState.Idle,
    val caption: CaptionState = CaptionState(),
)

class MainViewModel(
    private val client: SessionClient,
    private val secrets: SecretStore,
    private val microphone: MicrophoneCapture,
) : ViewModel() {
    private val initialKey = secrets.loadApiKey()
    private val mutableState = MutableStateFlow(
        MainUiState(apiKey = initialKey.orEmpty(), rememberKey = initialKey != null),
    )
    val state: StateFlow<MainUiState> = mutableState.asStateFlow()

    init {
        viewModelScope.launch {
            client.events.collect { event ->
                when (event) {
                    ClientEvent.Connected -> {
                        mutableState.update { it.copy(session = SessionState.Running) }
                        microphone.start(
                            scope = viewModelScope,
                            onFrame = { frame ->
                                try { client.sendAudio(frame) } finally { frame.fill(0) }
                            },
                            onError = { message ->
                                client.stop()
                                mutableState.update { it.copy(session = SessionState.Failed(message)) }
                            },
                        )
                    }
                    is ClientEvent.Caption -> mutableState.update { it.copy(caption = event.value) }
                    is ClientEvent.Failed -> {
                        microphone.stop()
                        mutableState.update { it.copy(session = SessionState.Failed(event.safeMessage)) }
                    }
                    ClientEvent.Closed -> {
                        microphone.stop()
                        mutableState.update { it.copy(session = SessionState.Idle) }
                    }
                }
            }
        }
    }

    fun setApiKey(value: String) = mutableState.update { it.copy(apiKey = value.take(512)) }
    fun setTargetLanguage(value: String) = mutableState.update { it.copy(targetLanguage = value.take(24)) }
    fun setMode(value: GeminiMode) = mutableState.update { it.copy(mode = value, caption = CaptionState()) }
    fun setRememberKey(value: Boolean) = mutableState.update { it.copy(rememberKey = value) }

    fun start() {
        val snapshot = mutableState.value
        if (snapshot.session !is SessionState.Idle && snapshot.session !is SessionState.Failed) return
        val config = runCatching { SessionConfig(snapshot.apiKey, snapshot.mode, snapshot.targetLanguage).validated() }
            .getOrElse { error ->
                mutableState.update { it.copy(session = SessionState.Failed(error.message ?: "配置无效")) }
                return
            }
        if (snapshot.rememberKey) secrets.saveApiKey(config.apiKey) else secrets.clear()
        mutableState.update { it.copy(session = SessionState.Connecting, caption = CaptionState()) }
        runCatching { client.connect(config) }.onFailure {
            mutableState.update { state -> state.copy(session = SessionState.Failed("无法启动会话")) }
        }
    }

    fun stop() {
        mutableState.update { it.copy(session = SessionState.Stopping) }
        microphone.stop()
        client.stop()
    }

    fun microphonePermissionDenied() {
        mutableState.update { it.copy(session = SessionState.Failed("需要麦克风权限才能开始")) }
    }

    override fun onCleared() {
        microphone.stop()
        client.stop()
        super.onCleared()
    }
}
