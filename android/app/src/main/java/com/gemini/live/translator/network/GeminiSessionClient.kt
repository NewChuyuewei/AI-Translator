package com.gemini.live.translator.network

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

interface SessionClient {
    val events: SharedFlow<ClientEvent>
    fun connect(config: SessionConfig)
    fun stop()
}

sealed interface ClientEvent {
    data object Connected : ClientEvent
    data class Caption(val value: CaptionState) : ClientEvent
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
        val candidate = http.newWebSocket(request, Listener(checked))
        if (!socket.compareAndSet(null, candidate)) candidate.cancel()
    }

    override fun stop() {
        socket.getAndSet(null)?.close(1000, "client stop")
    }

    private inner class Listener(private val config: SessionConfig) : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (socket.get() !== webSocket) return
            webSocket.send(setup(config).toString())
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > MAX_MESSAGE_CHARS) {
                fail(webSocket, "服务器消息超过安全限制")
                return
            }
            parse(text)?.let(mutableEvents::tryEmit)
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

    private fun parse(text: String): ClientEvent? = runCatching {
        val root = JSONObject(text)
        if (root.has("setupComplete")) return ClientEvent.Connected
        val content = root.optJSONObject("serverContent") ?: root.optJSONObject("server_content") ?: return null
        fun transcription(camel: String, snake: String): String =
            (content.optJSONObject(camel) ?: content.optJSONObject(snake))
                ?.optString("text")
                ?.take(MAX_TRANSCRIPT_CHARS)
                .orEmpty()
        val input = transcription("inputTranscription", "input_transcription")
        val output = transcription("outputTranscription", "output_transcription")
        if (input.isEmpty() && output.isEmpty()) null else ClientEvent.Caption(CaptionState(input, output))
    }.getOrElse { ClientEvent.Failed("Gemini 返回了无效消息") }

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
    }
}
