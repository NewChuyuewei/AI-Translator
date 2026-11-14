package com.gemini.live.translator

import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import com.gemini.live.translator.audio.AndroidAudioRouter
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
import java.util.concurrent.atomic.AtomicReference

class SessionRuntime(context: Context) {
    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val client: SessionClient = GeminiSessionClient()
    private val audioRouter = AndroidAudioRouter()
    private val player = TranslatedAudioPlayer()
    private val activeConfig = AtomicReference<SessionConfig?>(null)
    private val pendingProjectionData = AtomicReference<Intent?>(null)
    private val mutableSession = MutableStateFlow<SessionState>(SessionState.Idle)
    private val mutableCaption = MutableStateFlow(CaptionState())
    val session: StateFlow<SessionState> = mutableSession.asStateFlow()
    val caption: StateFlow<CaptionState> = mutableCaption.asStateFlow()

    init {
        scope.launch {
            client.events.collect { event ->
                when (event) {
                    ClientEvent.Connected -> {
                        val config = activeConfig.get() ?: run {
                            fail("会话配置已失效")
                            return@collect
                        }
                        val mediaProjection = if (config.audioSource.usesDeviceAudio) {
                            val projectionData = pendingProjectionData.getAndSet(null) ?: run {
                                fail("手机内声音授权已失效")
                                return@collect
                            }
                            val manager = appContext.getSystemService(MediaProjectionManager::class.java)
                            runCatching {
                                requireNotNull(manager.getMediaProjection(android.app.Activity.RESULT_OK, projectionData))
                            }.getOrElse {
                                    fail("无法创建手机内声音捕获会话")
                                    return@collect
                                }
                        } else {
                            null
                        }
                        mutableSession.value = SessionState.Running
                        val audioStarted = runCatching {
                            audioRouter.start(
                                scope = scope,
                                mode = config.audioSource,
                                mediaProjection = mediaProjection,
                                onFrame = { frame ->
                                    try { client.sendAudio(frame) } finally { frame.fill(0) }
                                },
                                onError = ::fail,
                            )
                        }
                        if (audioStarted.isFailure) {
                            fail("无法启动所选音频来源")
                            return@collect
                        }
                        if (mutableSession.value is SessionState.Running && !config.mode.transcriptionOnly) {
                            player.start(scope, ::fail)
                        }
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
    fun start(config: SessionConfig, projectionData: Intent? = null) {
        check(mutableSession.value is SessionState.Idle || mutableSession.value is SessionState.Failed)
        val checked = config.validated()
        if (checked.audioSource.usesDeviceAudio) requireNotNull(projectionData) { "需要手机内声音授权" }
        activeConfig.set(checked)
        pendingProjectionData.set(projectionData)
        mutableCaption.value = CaptionState()
        mutableSession.value = SessionState.Connecting
        runCatching {
            SessionForegroundService.start(appContext, checked.audioSource)
            client.connect(checked)
        }.onFailure { fail("无法启动会话") }
    }

    @Synchronized
    fun stop() {
        if (mutableSession.value is SessionState.Idle) return
        mutableSession.value = SessionState.Stopping
        audioRouter.stop()
        pendingProjectionData.set(null)
        activeConfig.set(null)
        player.stop()
        client.stop()
        finish()
    }

    private fun fail(message: String) {
        audioRouter.stop()
        pendingProjectionData.set(null)
        activeConfig.set(null)
        player.stop()
        client.stop()
        mutableSession.value = SessionState.Failed(message)
        SessionForegroundService.stop(appContext)
    }

    private fun finish() {
        audioRouter.stop()
        pendingProjectionData.set(null)
        activeConfig.set(null)
        player.stop()
        mutableSession.update { if (it is SessionState.Failed) it else SessionState.Idle }
        SessionForegroundService.stop(appContext)
    }
}
