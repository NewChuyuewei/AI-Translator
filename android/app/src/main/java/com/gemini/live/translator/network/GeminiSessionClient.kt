package com.gemini.live.translator.network

import com.gemini.live.translator.audio.PCM_FRAME_BYTES
import com.gemini.live.translator.data.CaptionState
import com.gemini.live.translator.data.GeminiMode
import com.gemini.live.translator.data.SessionConfig
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONObject
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import android.util.Base64

interface SessionClient {
    val events: SharedFlow<ClientEvent>
    fun connect(config: SessionConfig)
    fun sendAudio(frame: ByteArray): Boolean
    fun stop()
}

sealed interface ClientEvent {
    data object Connected : ClientEvent
    data class Caption(val value: CaptionState) : ClientEvent
    data class Audio(val pcm: ByteArray) : ClientEvent
    data class Failed(val safeMessage: String) : ClientEvent
    data object Closed : ClientEvent
}

class GeminiSessionClient : SessionClient {
    private val http = OkHttpClient.Builder()
        .connectTimeout(20, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .retryOnConnectionFailure(true)
        .build()
    private enum class ConnectionState { IDLE, CONNECTING, AWAITING_SETUP, READY }

    private val socket = AtomicReference<WebSocket?>(null)
    private val connectionState = AtomicReference(ConnectionState.IDLE)
    private val scheduler = ScheduledThreadPoolExecutor(1) { runnable ->
        Thread(runnable, "gemini-setup-timeout").apply { isDaemon = true }
    }.apply { removeOnCancelPolicy = true }
    private val mutableEvents = MutableSharedFlow<ClientEvent>(
        extraBufferCapacity = 32,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    override val events: SharedFlow<ClientEvent> = mutableEvents

    override fun connect(config: SessionConfig) {
        check(connectionState.compareAndSet(ConnectionState.IDLE, ConnectionState.CONNECTING)) { "session already active" }
        val checked = runCatching { config.validated() }.getOrElse {
            connectionState.set(ConnectionState.IDLE)
            throw it
        }
        val request = Request.Builder()
            .url(ENDPOINT)
            .header("x-goog-api-key", checked.apiKey)
            .build()
        val setupMessage = setup(checked).toString()
        val listener = Listener(setupMessage)
        val candidate = http.newWebSocket(request, listener)
        if (!socket.compareAndSet(null, candidate)) {
            connectionState.set(ConnectionState.IDLE)
            candidate.cancel()
            error("session already active")
        }
        listener.armTimeout(candidate)
    }

    override fun sendAudio(frame: ByteArray): Boolean {
        require(frame.size == PCM_FRAME_BYTES) { "audio frame must be 40 ms PCM16" }
        if (connectionState.get() != ConnectionState.READY) return false
        val active = socket.get() ?: return false
        if (active.queueSize() > MAX_SOCKET_QUEUE_BYTES) return false
        val encoded = Base64.encodeToString(frame, Base64.NO_WRAP)
        val message = JSONObject().put(
            "realtimeInput",
            JSONObject().put(
                "audio",
                JSONObject().put("mimeType", "audio/pcm;rate=16000").put("data", encoded),
            ),
        )
        return active.send(message.toString())
    }

    override fun stop() {
        val active = socket.getAndSet(null)
        if (active == null) {
            connectionState.set(ConnectionState.IDLE)
            return
        }
        // Mark idle immediately. Late callbacks belong to this detached socket
        // and are ignored by identity checks, so a deliberate stop cannot race
        // with the next user-initiated session.
        connectionState.set(ConnectionState.IDLE)
        if (!active.close(1000, "client stop")) active.cancel()
    }

    private inner class Listener(private val setupMessage: String) : WebSocketListener() {
        private val timeout = AtomicReference<ScheduledFuture<*>?>(null)

        fun armTimeout(webSocket: WebSocket) {
            val future = scheduler.schedule(
                { if (socket.get() === webSocket && connectionState.get() != ConnectionState.READY) fail(webSocket, "Gemini 初始化超时") },
                SETUP_TIMEOUT_SECONDS,
                TimeUnit.SECONDS,
            )
            if (!timeout.compareAndSet(null, future)) future.cancel(false)
        }

        private fun cancelTimeout() { timeout.getAndSet(null)?.cancel(false) }
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (socket.get() !== webSocket || !connectionState.compareAndSet(ConnectionState.CONNECTING, ConnectionState.AWAITING_SETUP)) return
            if (!webSocket.send(setupMessage)) fail(webSocket, "无法发送 Gemini 初始化配置")
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (socket.get() !== webSocket) return
            if (text.length > MAX_MESSAGE_CHARS) {
                fail(webSocket, "服务器消息超过安全限制")
                return
            }
            for (event in parse(text)) {
                if (event == ClientEvent.Connected) {
                    if (connectionState.compareAndSet(ConnectionState.AWAITING_SETUP, ConnectionState.READY)) {
                        cancelTimeout()
                        mutableEvents.tryEmit(event)
                    }
                } else if (connectionState.get() == ConnectionState.READY || event is ClientEvent.Failed) {
                    mutableEvents.tryEmit(event)
                }
            }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            if (bytes.size > MAX_MESSAGE_BYTES) {
                fail(webSocket, "服务器消息超过安全限制")
                return
            }
            val text = runCatching { bytes.toByteArray().decodeToString(throwOnInvalidSequence = true) }.getOrNull() ?: run {
                fail(webSocket, "服务器消息编码无效")
                return
            }
            onMessage(webSocket, text)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            cancelTimeout()
            if (!socket.compareAndSet(webSocket, null)) return
            connectionState.set(ConnectionState.IDLE)
            // Never expose response bodies, URLs, headers, or exception text: they may contain secrets.
            val message = when {
                response != null -> "Gemini 连接被拒绝（HTTP ${response.code}）"
                t is java.net.SocketTimeoutException -> "Gemini 连接超时"
                t is javax.net.ssl.SSLException -> "TLS 证书验证失败"
                else -> "Gemini 网络连接失败"
            }
            mutableEvents.tryEmit(ClientEvent.Failed(message))
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            cancelTimeout()
            if (!socket.compareAndSet(webSocket, null)) return
            connectionState.set(ConnectionState.IDLE)
            mutableEvents.tryEmit(ClientEvent.Closed)
        }

        private fun fail(webSocket: WebSocket, message: String) {
            cancelTimeout()
            if (!socket.compareAndSet(webSocket, null)) return
            connectionState.set(ConnectionState.IDLE)
            webSocket.cancel()
            mutableEvents.tryEmit(ClientEvent.Failed(message))
        }
    }

    private fun parse(text: String): List<ClientEvent> = runCatching {
        val root = JSONObject(text)
        val serverError = root.optJSONObject("error")
        if (serverError != null) {
            val code = serverError.optInt("code", 0)
            return listOf(ClientEvent.Failed(if (code in 400..599) "Gemini 服务拒绝请求（代码 $code）" else "Gemini 服务返回错误"))
        }
        if (root.has("setupComplete") || root.has("setup_complete")) return listOf(ClientEvent.Connected)
        val content = root.optJSONObject("serverContent") ?: root.optJSONObject("server_content") ?: return emptyList()
        val events = mutableListOf<ClientEvent>()
        fun transcription(camel: String, snake: String): String =
            (content.optJSONObject(camel) ?: content.optJSONObject(snake))
                ?.optString("text")
                ?.take(MAX_TRANSCRIPT_CHARS)
                .orEmpty()
        val input = transcription("inputTranscription", "input_transcription")
        val output = transcription("outputTranscription", "output_transcription")
        if (input.isNotEmpty() || output.isNotEmpty()) events += ClientEvent.Caption(CaptionState(input, output))
        val turn = content.optJSONObject("modelTurn") ?: content.optJSONObject("model_turn")
        val parts = turn?.optJSONArray("parts")
        var totalAudio = 0
        if (parts != null) for (index in 0 until parts.length()) {
            val part = parts.optJSONObject(index) ?: continue
            val inline = part.optJSONObject("inlineData") ?: part.optJSONObject("inline_data") ?: continue
            val mime = inline.optString("mimeType", inline.optString("mime_type", ""))
            if (!mime.startsWith("audio/pcm") || !mime.contains("rate=24000")) continue
            val encoded = inline.optString("data")
            if (encoded.length > MAX_AUDIO_BASE64_CHARS) error("audio too large")
            val decoded = Base64.decode(encoded, Base64.NO_WRAP)
            totalAudio += decoded.size
            if (totalAudio > MAX_AUDIO_BYTES) error("audio too large")
            events += ClientEvent.Audio(decoded)
        }
        events
    }.getOrElse { listOf(ClientEvent.Failed("Gemini 返回了无效消息")) }

    private fun setup(config: SessionConfig): JSONObject {
        val generation = JSONObject().put(
            "responseModalities",
            org.json.JSONArray().put(if (config.mode.transcriptionOnly) "TEXT" else "AUDIO"),
        )
        val setup = JSONObject()
            .put("model", config.mode.wireModel)
            .put("generationConfig", generation)
        when (config.mode) {
            GeminiMode.LIVE_TRANSLATE -> {
                generation.put(
                    "translationConfig",
                    JSONObject()
                        .put("targetLanguageCode", config.targetLanguage)
                        .put("echoTargetLanguage", false),
                )
                setup.put("inputAudioTranscription", JSONObject())
                    .put("outputAudioTranscription", JSONObject())
            }
            GeminiMode.LIVE_TRANSCRIBE -> setup.put(
                "inputAudioTranscription",
                JSONObject()
                    .put("languageCodes", org.json.JSONArray())
                    .put("mode", "VERBATIM"),
            )
            GeminiMode.GENERAL_LIVE, GeminiMode.EXTENDED_THINKING -> {
                if (config.mode == GeminiMode.EXTENDED_THINKING) {
                    generation.put("thinkingConfig", JSONObject().put("thinkingLevel", "MEDIUM"))
                }
                val target = languageInstructionName(config.targetLanguage)
                val instruction = "Act only as a low-latency simultaneous interpreter. " +
                    "Translate every heard utterance into $target. " +
                    "Start speaking the translation as soon as a meaningful phrase is available instead of waiting for a full paragraph. " +
                    "Output only $target. Never repeat or transcribe the source language, answer questions, explain, " +
                    "add commentary, or mix source and target languages."
                setup.put(
                    "realtimeInputConfig",
                    JSONObject().put(
                        "automaticActivityDetection",
                        JSONObject()
                            .put("disabled", false)
                            .put("startOfSpeechSensitivity", "START_SENSITIVITY_HIGH")
                            .put("endOfSpeechSensitivity", "END_SENSITIVITY_HIGH")
                            .put("prefixPaddingMs", 40)
                            .put("silenceDurationMs", 400),
                    ),
                ).put(
                    "systemInstruction",
                    JSONObject().put(
                        "parts",
                        org.json.JSONArray().put(JSONObject().put("text", instruction)),
                    ),
                ).put("inputAudioTranscription", JSONObject())
                    .put("outputAudioTranscription", JSONObject())
            }
        }
        return JSONObject().put("setup", setup)
    }

    private fun languageInstructionName(code: String): String {
        val name = when (code.lowercase()) {
            "zh", "zh-hans", "cmn-hans" -> "Simplified Chinese"
            "zh-hant", "cmn-hant" -> "Traditional Chinese"
            "en", "en-us", "en-gb" -> "English"
            "ja", "ja-jp" -> "Japanese"
            "ko", "ko-kr" -> "Korean"
            "es" -> "Spanish"
            "fr" -> "French"
            "de" -> "German"
            "it" -> "Italian"
            else -> return "the language identified by BCP-47 code $code"
        }
        return "$name ($code)"
    }

    private companion object {
        const val SETUP_TIMEOUT_SECONDS = 25L
        const val ENDPOINT = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
        const val MAX_MESSAGE_BYTES = 2 * 1024 * 1024
        const val MAX_MESSAGE_CHARS = 2 * 1024 * 1024
        const val MAX_TRANSCRIPT_CHARS = 16_000
        const val MAX_AUDIO_BYTES = 1024 * 1024
        const val MAX_AUDIO_BASE64_CHARS = (MAX_AUDIO_BYTES / 3 + 1) * 4
        const val MAX_SOCKET_QUEUE_BYTES = 32L * 1024
    }
}
