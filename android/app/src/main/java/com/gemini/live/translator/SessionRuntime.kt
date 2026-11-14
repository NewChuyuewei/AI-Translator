package com.gemini.live.translator

import android.content.Context
import com.gemini.live.translator.audio.MicrophoneCapture
import com.gemini.live.translator.audio.TranslatedAudioPlayer
import com.gemini.live.translator.data.CaptionState
import com.gemini.live.translator.data.SessionConfig
import com.gemini.live.translator.data.SessionState
import com.gemini.live.translator.network.ClientEvent
import com.gemini.live.translator.network.GeminiSessionClient
import com.gemini.live.translator.network.SessionClient
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

class SessionRuntime(context: Context) {
    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val client: SessionClient = GeminiSessionClient()
    private val microphone = MicrophoneCapture()
    private val player = TranslatedAudioPlayer()
    private val mutableSession = MutableStateFlow<SessionState>(SessionState.Idle)
    private val mutableCaption = MutableStateFlow(CaptionState())
    val session: StateFlow<SessionState> = mutableSession.asStateFlow()
    val caption: StateFlow<CaptionState> = mutableCaption.asStateFlow()

    init {
        scope.launch {
            client.events.collect { event ->
                when (event) {
                    ClientEvent.Connected -> {
                        mutableSession.value = SessionState.Running
                        microphone.start(
                            scope,
                            onFrame = { frame ->
                                try { client.sendAudio(frame) } finally { frame.fill(0) }
                            },
                            onError = ::fail,
                        )
                        player.start(scope, ::fail)
                    }
                    is ClientEvent.Caption -> mutableCaption.value = event.value
                    is ClientEvent.Audio -> player.offer(event.pcm).also { event.pcm.fill(0) }
                    is ClientEvent.Failed -> fail(event.safeMessage)
                    ClientEvent.Closed -> finish()
                }
            }
        }
    }

    @Synchronized
    fun start(config: SessionConfig) {
        check(mutableSession.value is SessionState.Idle || mutableSession.value is SessionState.Failed)
        mutableCaption.value = CaptionState()
        mutableSession.value = SessionState.Connecting
        SessionForegroundService.start(appContext)
        runCatching { client.connect(config.validated()) }.onFailure { fail("无法启动会话") }
    }

    @Synchronized
    fun stop() {
        if (mutableSession.value is SessionState.Idle) return
        mutableSession.value = SessionState.Stopping
        microphone.stop()
        player.stop()
        client.stop()
        finish()
    }

    private fun fail(message: String) {
        microphone.stop()
        player.stop()
        client.stop()
        mutableSession.value = SessionState.Failed(message)
        SessionForegroundService.stop(appContext)
    }

    private fun finish() {
        microphone.stop()
        player.stop()
        mutableSession.update { if (it is SessionState.Failed) it else SessionState.Idle }
        SessionForegroundService.stop(appContext)
    }
}
