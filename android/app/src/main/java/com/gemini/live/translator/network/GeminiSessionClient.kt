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
    private val socket = AtomicReference<WebSocket?>(null)
    private val mutableEvents = MutableSharedFlow<ClientEvent>(
        extraBufferCapacity = 32,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    override val events: SharedFlow<ClientEvent> = mutableEvents

    override fun connect(config: SessionConfig) {
        check(socket.get() == null) { "session already active" }
        val checked = config.validated()
        val request = Request.Builder()
            .url(ENDPOINT)
            .header("x-goog-api-key", checked.apiKey)
            .build()
        val setupMessage = setup(checked).toString()
        val candidate = http.newWebSocket(request, Listener(setupMessage))
        if (!socket.compareAndSet(null, candidate)) candidate.cancel()
    }

    override fun sendAudio(frame: ByteArray): Boolean {
        require(frame.size == PCM_FRAME_BYTES) { "audio frame must be 40 ms PCM16" }
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
        socket.getAndSet(null)?.close(1000, "client stop")
    }

    private inner class Listener(private val setupMessage: String) : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (socket.get() !== webSocket) return
            webSocket.send(setupMessage)
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > MAX_MESSAGE_CHARS) {
                fail(webSocket, "服务器消息超过安全限制")
                return
            }
            parse(text).forEach(mutableEvents::tryEmit)
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
            socket.compareAndSet(webSocket, null)
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
            socket.compareAndSet(webSocket, null)
            mutableEvents.tryEmit(ClientEvent.Closed)
        }

        private fun fail(webSocket: WebSocket, message: String) {
            mutableEvents.tryEmit(ClientEvent.Failed(message))
            webSocket.cancel()
            socket.compareAndSet(webSocket, null)
        }
    }

    private fun parse(text: String): List<ClientEvent> = runCatching {
        val root = JSONObject(text)
        if (root.has("setupComplete")) return listOf(ClientEvent.Connected)
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
        val generation = JSONObject().put("responseModalities", if (config.mode.transcriptionOnly) org.json.JSONArray().put("TEXT") else org.json.JSONArray().put("AUDIO"))
        if (config.mode == GeminiMode.LIVE_TRANSLATE) {
            generation.put("translationConfig", JSONObject().put("targetLanguageCode", config.targetLanguage).put("echoTargetLanguage", false))
        }
        return JSONObject().put("setup", JSONObject()
            .put("model", config.mode.wireModel)
            .put("generationConfig", generation)
            .put("inputAudioTranscription", JSONObject()))
    }

    private companion object {
        const val ENDPOINT = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
        const val MAX_MESSAGE_BYTES = 2 * 1024 * 1024
        const val MAX_MESSAGE_CHARS = 2 * 1024 * 1024
        const val MAX_TRANSCRIPT_CHARS = 16_000
        const val MAX_AUDIO_BYTES = 1024 * 1024
        const val MAX_AUDIO_BASE64_CHARS = (MAX_AUDIO_BYTES / 3 + 1) * 4
        const val MAX_SOCKET_QUEUE_BYTES = 32L * 1024
    }
}
