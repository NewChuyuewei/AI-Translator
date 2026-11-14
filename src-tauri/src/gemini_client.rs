//! Gemini Live raw WebSocket protocol. API key is sent in a request header,
//! never embedded in the URL or logged.
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, time::{Duration, Instant}};
use tokio::{net::TcpStream, sync::watch};
use tokio_tungstenite::{client_async_tls_with_config, tungstenite::{client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig, Error as WsError, Message}, MaybeTlsStream, WebSocketStream};
use crate::{diagnostics::Trace, proxy::{self, Route}};

const ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const MAX_SERVER_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;
const MAX_AUDIO_BYTES_PER_MESSAGE: usize = 1024 * 1024;
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    GeneralLive,
    // Preserve compatibility with the option used by the previous release.
    #[serde(alias = "live_translate")]
    ExtendedThinking,
    #[serde(rename = "live_translate_35")]
    LiveTranslate35,
    #[serde(rename = "transcribe_live_35")]
    TranscribeLive35,
}
impl ModelKind {
    pub fn is_transcription_only(self) -> bool { self == Self::TranscribeLive35 }
    pub fn supports_bidirectional_instruction(self) -> bool {
        matches!(self, Self::GeneralLive | Self::ExtendedThinking)
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThinkingLevel { #[default] Low, Medium, High }
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TranscriptionMode { #[default] Verbatim, Smart }
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiOptions {
    pub api_key: String,
    pub model: ModelKind,
    #[serde(default)] pub thinking_level: ThinkingLevel,
    #[serde(default)] pub transcription_mode: TranscriptionMode,
    pub target_language: String,
    #[serde(default)] pub bidirectional: bool,
    #[serde(default = "default_other_language")] pub other_language: String,
}
fn default_other_language() -> String { "en".into() }
fn language_instruction_name(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "zh" | "zh-hans" | "cmn-hans" => "Simplified Chinese",
        "zh-hant" | "cmn-hant" => "Traditional Chinese",
        "en" | "en-us" | "en-gb" => "English",
        "ja" | "ja-jp" => "Japanese",
        "ko" | "ko-kr" => "Korean",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        _ => return format!("the language identified by BCP-47 code {code}"),
    };
    format!("{name} ({code})")
}
fn valid_language(value: &str) -> bool {
    (2..=24).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
}
impl GeminiOptions {
    pub fn validate(&self) -> Result<()> {
        let key = self.api_key.trim();
        if key.is_empty() { return Err(anyhow!("请填写 Gemini API Key")); }
        if key.len() > 512 { return Err(anyhow!("Gemini API Key 长度无效")); }
        if !valid_language(&self.target_language) || !valid_language(&self.other_language) { return Err(anyhow!("语言代码必须是有效的 BCP-47 字母/连字符形式")); }
        if self.bidirectional && !self.model.supports_bidirectional_instruction() {
            return Err(anyhow!("此模型不支持双向自动切换；请选择单向翻译或转写"));
        }
        Ok(())
    }
}

pub fn setup(options: &GeminiOptions) -> Value {
    match options.model {
        ModelKind::LiveTranslate35 => {
            // The translation model accepts one fixed destination language per
            // session. Google's supported Chinese codes distinguish scripts.
            let language = if options.target_language.eq_ignore_ascii_case("zh") { "zh-Hans" } else { &options.target_language };
            json!({"setup": {
                "model": "models/gemini-3.5-live-translate-preview",
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "translationConfig": {"targetLanguageCode": language, "echoTargetLanguage": false}
                },
                // For raw v1beta WebSockets these are setup fields, not
                // generationConfig fields; nesting them there is rejected.
                "inputAudioTranscription": {}, "outputAudioTranscription": {}
            }})
        }
        ModelKind::TranscribeLive35 => json!({"setup": {
            "model": "models/gemini-3.5-transcribe-live",
            "generationConfig": {"responseModalities": ["TEXT"]},
            // An empty language list enables multilingual auto-detection.
            "inputAudioTranscription": {"languageCodes": [], "mode": options.transcription_mode}
        }}),
        ModelKind::GeneralLive | ModelKind::ExtendedThinking => {
            let target = language_instruction_name(&options.target_language);
            let other = language_instruction_name(&options.other_language);
            let instruction = if options.bidirectional {
                format!("Act only as a low-latency simultaneous interpreter. Translate speech in {target} into {other}, and speech in {other} into {target}. Start each translation as soon as a meaningful phrase is available. Output only the translation in the opposite language. Never repeat or transcribe the source language, answer questions, explain, add commentary, or mix source and target languages.")
            } else {
                format!("Act only as a low-latency simultaneous interpreter. Translate every heard utterance into {target}. Start speaking the translation as soon as a meaningful phrase is available instead of waiting for a full paragraph. Output only {target}. Never repeat or transcribe the source language, answer questions, explain, add commentary, or mix source and target languages.")
            };
            let (model, generation) = match options.model {
                ModelKind::GeneralLive => ("models/gemini-3.8-live", json!({"responseModalities": ["AUDIO"]})),
                ModelKind::ExtendedThinking => ("models/gemini-3.8-live-extended-thinking", json!({
                    "responseModalities": ["AUDIO"], "thinkingConfig": {"thinkingLevel": options.thinking_level}
                })),
                _ => unreachable!(),
            };
            json!({"setup": {
                "model": model, "generationConfig": generation,
                "systemInstruction": {"parts": [{"text": instruction}]},
                "realtimeInputConfig": {"automaticActivityDetection": {
                    "disabled": false,
                    "startOfSpeechSensitivity": "START_SENSITIVITY_HIGH",
                    "endOfSpeechSensitivity": "END_SENSITIVITY_HIGH",
                    "prefixPaddingMs": 40,
                    "silenceDurationMs": 400
                }},
                "inputAudioTranscription": {}, "outputAudioTranscription": {}
            }})
        }
    }
}
pub fn audio_frame(data: &[u8]) -> Message {
    Message::Text(json!({"realtimeInput": {"audio": {"mimeType": "audio/pcm;rate=16000", "data": STANDARD.encode(data)}}}).to_string().into())
}
pub fn stream_end() -> Message { Message::Text(json!({"realtimeInput": {"audioStreamEnd": true}}).to_string().into()) }

pub async fn connect(options: &GeminiOptions, route: &Route, cancel: &mut watch::Receiver<bool>, trace: &Trace) -> Result<Socket> {
    connect_with_trace(ENDPOINT, options, route, cancel, None, trace).await
}
#[cfg(test)]
async fn connect_to(endpoint: &str, options: &GeminiOptions, route: &Route, cancel: &mut watch::Receiver<bool>) -> Result<Socket> {
    connect_using(endpoint, options, route, cancel, None).await
}
#[cfg(test)]
async fn connect_using(endpoint: &str, options: &GeminiOptions, route: &Route, cancel: &mut watch::Receiver<bool>, connector: Option<tokio_tungstenite::Connector>) -> Result<Socket> {
    connect_with_trace(endpoint, options, route, cancel, connector, &Trace::silent()).await
}

/// Describe transport failures without dumping HTTP bodies, headers or payloads.
pub fn describe_ws_error(error: &WsError) -> String {
    match error {
        WsError::Http(response) => format!("HTTP 状态码 {}", response.status().as_u16()),
        WsError::Tls(_) => "TLS 握手失败；请检查证书和代理".into(),
        WsError::Io(e) if e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()).is_some() =>
            "TLS 握手或证书验证失败；请检查代理是否解密 TLS".into(),
        WsError::Io(e) => match e.raw_os_error() {
            Some(code) => format!("网络 I/O 错误（{:?}, 系统代码 {}）", e.kind(), code),
            None => format!("网络 I/O 错误（{:?}）", e.kind()),
        },
        WsError::Protocol(_) => "WebSocket 协议异常".into(),
        WsError::Capacity(_) => "WebSocket 消息超过大小限制".into(),
        WsError::Utf8 => "WebSocket 文本不是有效 UTF-8".into(),
        WsError::ConnectionClosed | WsError::AlreadyClosed => "WebSocket 已关闭".into(),
        WsError::WriteBufferFull(_) => "WebSocket 发送缓冲已满".into(),
        WsError::AttackAttempt => "WebSocket 检测到无效握手".into(),
        WsError::Url(_) => "WebSocket 地址无效".into(),
        WsError::HttpFormat(_) => "HTTP 握手格式无效".into(),
    }
}

// Production uses None: rustls/webpki's normal certificate roots. The test-only
// caller can instead supply a connector that trusts an ephemeral local CA.
async fn connect_with_trace(endpoint: &str, options: &GeminiOptions, route: &Route, cancel: &mut watch::Receiver<bool>, connector: Option<tokio_tungstenite::Connector>, trace: &Trace) -> Result<Socket> {
    options.validate()?;
    let mut request = endpoint.into_client_request().context("无效的 WebSocket 地址")?;
    request.headers_mut().insert("x-goog-api-key", HeaderValue::from_str(options.api_key.trim()).context("API Key 中含无效字符")?);
    let host = request.uri().host().ok_or_else(|| anyhow!("WebSocket 目标缺少主机"))?.to_owned();
    let port = request.uri().port_u16().unwrap_or(if request.uri().scheme_str() == Some("wss") { 443 } else { 80 });
    let connection_started = Instant::now();
    trace.info("开始建立 TCP 连接和代理隧道（总超时 25 秒）");
    let handshake = async {
        let tcp = proxy::open(route, &host, port).await?;
        trace.info(format!("网络路径已建立，耗时 {} ms", connection_started.elapsed().as_millis()));
        trace.info("开始 TLS / WebSocket 握手（验证服务端证书）");
        // TLS and certificate/hostname validation are performed AFTER the
        // proxy tunnel is established, before the API-key header is sent.
        let mut websocket = WebSocketConfig::default();
        websocket.max_message_size = Some(MAX_SERVER_MESSAGE_BYTES);
        websocket.max_frame_size = Some(MAX_SERVER_MESSAGE_BYTES);
        client_async_tls_with_config(request, tcp, Some(websocket), connector).await
            .map_err(|e| anyhow!("Gemini WebSocket/TLS 握手失败：{}", describe_ws_error(&e)))
    };
    let (mut socket, response) = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(25), handshake) => match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => { trace.error(format!("网络连接或握手失败（{} ms）：{}", connection_started.elapsed().as_millis(), e)); return Err(e); },
            Err(_) => { trace.error("连接 Gemini 超时（25 秒）；请检查网络和代理"); return Err(anyhow!("连接 Gemini 超时；请检查网络和代理")); },
        },
        _ = cancelled(cancel) => { trace.info("连接阶段已取消"); return Err(anyhow!("已停止")); },
    };
    trace.info(format!("WebSocket 握手成功：HTTP {}，总耗时 {} ms", response.status().as_u16(), connection_started.elapsed().as_millis()));
    trace.info("发送 setup（不会记录密钥、请求体或音频）");
    tokio::select! {
        sent = tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Text(setup(options).to_string().into()))) => match sent {
            Ok(Ok(())) => {},
            Ok(Err(e)) => { let message = format!("发送 setup 失败：{}", describe_ws_error(&e)); trace.error(&message); return Err(anyhow!(message)); },
            Err(_) => { trace.error("发送 setup 超时（10 秒）"); return Err(anyhow!("发送 setup 超时")); },
        },
        _ = cancelled(cancel) => { trace.info("发送 setup 阶段已取消"); return Err(anyhow!("已停止")); },
    }
    // Do not send audio before setupComplete. Gemini can return either a text
    // frame or a binary frame containing UTF-8 JSON; decode both identically.
    let setup_started = Instant::now();
    trace.info("setup 已发送，等待 setupComplete（最长 25 秒）");
    let wait = async {
        let mut seen = HashSet::new();
        while let Some(message) = socket.next().await {
            let message = message.map_err(|e| anyhow!("等待 setupComplete 时连接断开：{}", describe_ws_error(&e)))?;
            match decode_server_frame(message)? {
                ServerFrame::Data { kind, value } => {
                    let category = classify_message(&value);
                    if seen.insert((kind, category)) { trace.info(format!("初始化收到 {kind} 帧，类型 {category}")); }
                    if let Some(error) = value.get("error") { return Err(server_error(error, "Gemini setup 被拒绝")); }
                    if field(&value, "setupComplete", "setup_complete").is_some() { return Ok(()); }
                }
                ServerFrame::Close { code, hint } => return Err(anyhow!(close_description(code, hint))),
                ServerFrame::Control(kind) => {
                    if seen.insert(("控制", kind)) { trace.info(format!("初始化收到 WebSocket {kind}")); }
                }
            }
        }
        Err(anyhow!("Gemini 在 setupComplete 前关闭了连接"))
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(25), wait) => match result {
            Ok(Ok(())) => trace.info(format!("已收到 setupComplete，等待 {} ms", setup_started.elapsed().as_millis())),
            Ok(Err(e)) => { trace.error(format!("初始化失败（{} ms）：{}", setup_started.elapsed().as_millis(), e)); return Err(e); },
            Err(_) => { trace.error("等待 Gemini setupComplete 超时（25 秒）；未收到可识别的 setupComplete"); return Err(anyhow!("等待 Gemini setupComplete 超时")); },
        },
        _ = cancelled(cancel) => { trace.info("等待 setupComplete 阶段已取消"); return Err(anyhow!("已停止")); },
    }
    Ok(socket)
}
pub async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() { return; }
    while cancel.changed().await.is_ok() { if *cancel.borrow() { break; } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionStatus { InProgress, Idle }
#[derive(Default, Debug)]
pub struct ParsedMessage {
    pub input: Option<(String, bool)>, pub interim_input: Option<(String, bool)>,
    pub output: Option<(String, bool)>,
    pub audio: Vec<Vec<u8>>, pub interrupted: bool, pub turn_complete: bool, pub go_away: bool,
    pub interaction_status: Option<InteractionStatus>,
}
fn field<'a>(v: &'a Value, a: &str, b: &str) -> Option<&'a Value> { v.get(a).or_else(|| v.get(b)) }

/// One decoder for setup and the live session: text or binary UTF-8 JSON.
/// Neither parse errors nor diagnostics include the untrusted frame contents.
pub enum ServerFrame {
    Data { kind: &'static str, value: Value },
    Close { code: Option<u16>, hint: Option<&'static str> },
    Control(&'static str),
}
pub fn decode_server_frame(message: Message) -> Result<ServerFrame> {
    let (kind, value): (&'static str, Value) = match message {
        Message::Text(text) => {
            if text.len() > MAX_SERVER_MESSAGE_BYTES { return Err(anyhow!("Gemini 文本帧超过大小限制")); }
            ("文本", serde_json::from_str(&text).map_err(|e|
                anyhow!("Gemini 文本帧不是有效 JSON（行 {}，列 {}）", e.line(), e.column()))?)
        }
        Message::Binary(bytes) => {
            if bytes.len() > MAX_SERVER_MESSAGE_BYTES { return Err(anyhow!("Gemini 二进制帧超过大小限制")); }
            std::str::from_utf8(&bytes).map_err(|_| anyhow!("Gemini 二进制帧不是有效 UTF-8"))?;
            ("二进制", serde_json::from_slice(&bytes).map_err(|e|
                anyhow!("Gemini 二进制帧不是有效 JSON（行 {}，列 {}）", e.line(), e.column()))?)
        }
        Message::Close(frame) => return Ok(ServerFrame::Close {
            code: frame.as_ref().map(|f| u16::from(f.code)),
            hint: frame.as_ref().and_then(|f| safe_error_hint(&f.reason)),
        }),
        Message::Ping(_) => return Ok(ServerFrame::Control("Ping")),
        Message::Pong(_) => return Ok(ServerFrame::Control("Pong")),
        Message::Frame(_) => return Ok(ServerFrame::Control("其他控制帧")),
    };
    if !value.is_object() { return Err(anyhow!("Gemini {kind} 帧的 JSON 顶层不是对象")); }
    Ok(ServerFrame::Data { kind, value })
}

/// Only ever return fixed labels, never arbitrary JSON keys or values.
pub fn classify_message(value: &Value) -> &'static str {
    if value.get("error").is_some() { "error" }
    else if field(value, "setupComplete", "setup_complete").is_some() { "setupComplete" }
    else if field(value, "serverContent", "server_content").is_some() { "serverContent" }
    else if field(value, "goAway", "go_away").is_some() { "goAway" }
    else if field(value, "usageMetadata", "usage_metadata").is_some() { "usageMetadata" }
    else if field(value, "sessionResumptionUpdate", "session_resumption_update").is_some() { "sessionResumptionUpdate" }
    else if field(value, "interactionStatus", "interaction_status").is_some() { "interactionStatus" }
    else if field(value, "toolCall", "tool_call").is_some() { "toolCall" }
    else if field(value, "toolCallCancellation", "tool_call_cancellation").is_some() { "toolCallCancellation" }
    // Gemini may add forward-compatible metadata envelopes. They are valid
    // JSON but carry no audio/transcription state this client needs.
    else { "协议元数据（已安全忽略）" }
}

// Map untrusted server text to a fixed, generic hint; do not expose its reason,
// which might echo credentials or transcripts. Code and status are allowlisted.
fn safe_error_hint(raw: &str) -> Option<&'static str> {
    let s = raw.chars().take(512).collect::<String>().to_ascii_lowercase();
    if s.contains("api key") || s.contains("api_key") || s.contains("unauthenticated") || s.contains("permission_denied") || s.contains("permission denied") {
        Some("请检查 API Key 和账户权限")
    } else if s.contains("quota") || s.contains("resource_exhausted") || s.contains("rate limit") {
        Some("请检查配额和调用频率")
    } else if s.contains("not found") || s.contains("not_found") || s.contains("not supported") || s.contains("not available") || s.contains("unsupported model") {
        Some("请检查模型 ID、API 版本及账户可用性")
    } else if s.contains("region") || s.contains("location") || s.contains("country") {
        Some("请检查 Gemini 服务的地区可用性")
    } else if s.contains("invalid argument") || s.contains("invalid_argument") {
        Some("setup 参数可能不受支持")
    } else { None }
}
fn status_label(raw: &str) -> Option<&'static str> {
    match raw {
        "CANCELLED" => Some("CANCELLED"), "UNKNOWN" => Some("UNKNOWN"), "INVALID_ARGUMENT" => Some("INVALID_ARGUMENT"),
        "DEADLINE_EXCEEDED" => Some("DEADLINE_EXCEEDED"), "NOT_FOUND" => Some("NOT_FOUND"),
        "ALREADY_EXISTS" => Some("ALREADY_EXISTS"), "PERMISSION_DENIED" => Some("PERMISSION_DENIED"),
        "RESOURCE_EXHAUSTED" => Some("RESOURCE_EXHAUSTED"), "FAILED_PRECONDITION" => Some("FAILED_PRECONDITION"),
        "ABORTED" => Some("ABORTED"), "OUT_OF_RANGE" => Some("OUT_OF_RANGE"), "UNIMPLEMENTED" => Some("UNIMPLEMENTED"),
        "INTERNAL" => Some("INTERNAL"), "UNAVAILABLE" => Some("UNAVAILABLE"), "DATA_LOSS" => Some("DATA_LOSS"),
        "UNAUTHENTICATED" => Some("UNAUTHENTICATED"), _ => None,
    }
}
pub fn close_description(code: Option<u16>, hint: Option<&'static str>) -> String {
    let code = code.map(|c| c.to_string()).unwrap_or_else(|| "未提供".into());
    let suffix = hint.map(|h| format!("；{h}")).unwrap_or_default();
    format!("Gemini 关闭 WebSocket（关闭码 {code}）{suffix}；原始关闭原因未记录")
}
fn server_error(error: &Value, prefix: &str) -> anyhow::Error {
    let code = error.get("code").and_then(Value::as_u64).filter(|c| (100..=599).contains(c));
    let status = error.get("status").and_then(Value::as_str).and_then(status_label);
    let hint = error.get("message").and_then(Value::as_str).and_then(safe_error_hint)
        .or_else(|| status.and_then(safe_error_hint));
    let mut details = Vec::new();
    if let Some(code) = code { details.push(format!("代码 {code}")); }
    if let Some(status) = status { details.push(format!("状态 {status}")); }
    if let Some(hint) = hint { details.push(hint.into()); }
    if details.is_empty() { details.push("未提供可安全显示的错误代码".into()); }
    anyhow!("{prefix}（{}）；原始响应未记录", details.join("；"))
}
fn interaction_status(v: Option<&Value>) -> Option<InteractionStatus> {
    match v?.as_str()? {
        "IN_PROGRESS" => Some(InteractionStatus::InProgress),
        "IDLE" => Some(InteractionStatus::Idle),
        _ => None,
    }
}
fn transcription(v: Option<&Value>) -> Result<Option<(String, bool)>> {
    let Some(v) = v else { return Ok(None); };
    let Some(text) = v.get("text").and_then(Value::as_str) else { return Ok(None); };
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(anyhow!("Gemini 转写文本超过大小限制"));
    }
    let finished = v.get("finished").and_then(Value::as_bool).unwrap_or(false);
    Ok(Some((text.to_owned(), finished)))
}
#[cfg(test)]
pub fn parse_server_message(text: &str) -> Result<ParsedMessage> {
    match decode_server_frame(Message::Text(text.to_owned().into()))? {
        ServerFrame::Data { value, .. } => parse_server_value(&value),
        _ => Err(anyhow!("Gemini 消息不是 JSON 数据帧")),
    }
}
pub fn parse_server_value(root: &Value) -> Result<ParsedMessage> {
    if let Some(error) = root.get("error") { return Err(server_error(error, "Gemini 返回错误")); }
    let mut p = ParsedMessage {
        go_away: field(root, "goAway", "go_away").is_some(),
        interaction_status: interaction_status(field(root, "interactionStatus", "interaction_status")),
        ..Default::default()
    };
    if let Some(c) = field(root, "serverContent", "server_content") {
        p.interaction_status = interaction_status(field(c, "interactionStatus", "interaction_status")).or(p.interaction_status);
        p.input = transcription(field(c, "inputTranscription", "input_transcription"))?;
        p.interim_input = transcription(field(c, "interimInputTranscription", "interim_input_transcription"))?;
        p.output = transcription(field(c, "outputTranscription", "output_transcription"))?;
        p.interrupted = c.get("interrupted").and_then(Value::as_bool).unwrap_or(false);
        p.turn_complete = field(c,"turnComplete","turn_complete").and_then(Value::as_bool).unwrap_or(false);
        if let Some(parts) = field(c,"modelTurn","model_turn").and_then(|v| v.get("parts")).and_then(Value::as_array) {
            let mut audio_bytes = 0usize;
            for part in parts {
                if let Some(inline) = field(part,"inlineData","inline_data") {
                    if let Some(encoded) = inline.get("data").and_then(Value::as_str) {
                        let mime = field(inline,"mimeType","mime_type").and_then(Value::as_str).unwrap_or("audio/pcm;rate=24000");
                        if mime.starts_with("audio/pcm") {
                            if mime.contains("rate=") && !mime.contains("rate=24000") { return Err(anyhow!("不支持的 Gemini 输出采样率（需要 24000 Hz）")); }
                            // Bound encoded input before allocating its decoded form.
                            if encoded.len() > (MAX_AUDIO_BYTES_PER_MESSAGE / 3 + 1) * 4 {
                                return Err(anyhow!("Gemini 音频消息超过大小限制"));
                            }
                            let decoded = STANDARD.decode(encoded).map_err(|_| anyhow!("Gemini 音频 Base64 无效"))?;
                            audio_bytes = audio_bytes.checked_add(decoded.len()).ok_or_else(|| anyhow!("Gemini 音频消息超过大小限制"))?;
                            if audio_bytes > MAX_AUDIO_BYTES_PER_MESSAGE { return Err(anyhow!("Gemini 音频消息超过大小限制")); }
                            p.audio.push(decoded);
                        }
                    }
                }
            }
        }
    }
    Ok(p)
}

#[cfg(test)] mod tests {
    use super::*;
    fn opts(model: ModelKind) -> GeminiOptions { GeminiOptions { api_key: "test".into(), model, thinking_level: ThinkingLevel::Low, transcription_mode: TranscriptionMode::Verbatim, target_language: "zh".into(), bidirectional: false, other_language:"en".into() } }

    #[tokio::test]
    async fn binary_setup_and_binary_live_translation_work_on_real_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let _setup = ws.next().await.unwrap().unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(30), ws.next()).await.is_err());
            ws.send(Message::Binary(br#"{"setupComplete":{}}"#.to_vec().into())).await.unwrap();
            let translation = json!({"serverContent": {
                "inputTranscription": {"text": "PRIVATE-INPUT-TEXT"},
                "outputTranscription": {"text": "PRIVATE-TRANSLATION"},
                "modelTurn": {"parts": [{"inlineData": {"mimeType": "audio/pcm;rate=24000", "data": "AQIDBA=="}}]},
                "turnComplete": true
            }}).to_string();
            ws.send(Message::Binary(translation.into_bytes().into())).await.unwrap();
            ws.send(Message::Text(r#"{"server_content":{"output_transcription":{"text":"text frame works"}}}"#.into())).await.unwrap();
        });
        let mut options = opts(ModelKind::ExtendedThinking); options.api_key = "PRIVATE-API-KEY-123".into();
        let (trace, captured) = Trace::capturing(&options.api_key, "http://alice:secret-proxy@127.0.0.1:7890");
        let (_tx, mut rx) = watch::channel(false);
        let mut socket = tokio::time::timeout(Duration::from_secs(3), connect_with_trace(&format!("ws://{addr}"), &options, &proxy::direct_for_test(), &mut rx, None, &trace)).await.unwrap().unwrap();
        let frame = decode_server_frame(socket.next().await.unwrap().unwrap()).unwrap();
        let ServerFrame::Data { kind, value } = frame else { panic!("expected binary data frame") };
        assert_eq!(kind, "二进制"); assert_eq!(classify_message(&value), "serverContent");
        let parsed = parse_server_value(&value).unwrap();
        assert_eq!(parsed.input.unwrap().0, "PRIVATE-INPUT-TEXT");
        assert_eq!(parsed.output.unwrap().0, "PRIVATE-TRANSLATION");
        assert_eq!(parsed.audio, vec![vec![1, 2, 3, 4]]); assert!(parsed.turn_complete);
        let ServerFrame::Data { kind, value } = decode_server_frame(socket.next().await.unwrap().unwrap()).unwrap() else { panic!("expected text data frame") };
        assert_eq!(kind, "文本");
        assert_eq!(parse_server_value(&value).unwrap().output.unwrap().0, "text frame works");
        server.await.unwrap();
        let logs = captured.lock().unwrap().join("\n");
        assert!(logs.contains("二进制 帧，类型 setupComplete"));
        for secret in ["PRIVATE-API-KEY-123", "PRIVATE-INPUT-TEXT", "PRIVATE-TRANSLATION", "AQIDBA==", "alice", "secret-proxy"] {
            assert!(!logs.contains(secret), "diagnostics exposed {secret}");
        }
    }

    async fn rejected_setup(reply: Message) -> (String, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let _ = ws.next().await.unwrap().unwrap();
            ws.send(reply).await.unwrap();
        });
        let mut options = opts(ModelKind::GeneralLive); options.api_key = "PRIVATE-API-KEY-123".into();
        let (trace, captured) = Trace::capturing(&options.api_key, "http://alice:secret-proxy@localhost:7890");
        let (_tx, mut rx) = watch::channel(false);
        let error = tokio::time::timeout(Duration::from_secs(3), connect_with_trace(&format!("ws://{addr}"), &options, &proxy::direct_for_test(), &mut rx, None, &trace)).await.unwrap().err().expect("must reject setup");
        server.await.unwrap();
        let logs = captured.lock().unwrap().join("\n");
        (format!("{error:#}"), logs)
    }

    #[tokio::test]
    async fn malformed_binary_json_errors_and_close_are_safe_and_immediate() {
        use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
        let cases = [
            (Message::Binary(vec![0xff, 0xfe].into()), "不是有效 UTF-8"),
            (Message::Binary(b"not json PRIVATE-API-KEY-123".to_vec().into()), "不是有效 JSON"),
            (Message::Text(r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","message":"invalid argument PRIVATE-API-KEY-123 alice secret-proxy PRIVATE-INPUT-TEXT"}}"#.into()), "代码 400"),
            (Message::Binary(br#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"api key PRIVATE-API-KEY-123"}}"#.to_vec().into()), "代码 403"),
            (Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: "model not found PRIVATE-API-KEY-123 alice secret-proxy PRIVATE-INPUT-TEXT".into() })), "关闭码 1008"),
        ];
        for (reply, expected) in cases {
            let (error, logs) = rejected_setup(reply).await;
            assert!(error.contains(expected), "expected {expected}, got {error}");
            assert!(logs.contains(expected), "expected {expected} in {logs}");
            for secret in ["PRIVATE-API-KEY-123", "alice", "secret-proxy", "PRIVATE-INPUT-TEXT"] {
                assert!(!error.contains(secret), "error leaked {secret}");
                assert!(!logs.contains(secret), "log leaked {secret}");
            }
        }
        assert!(decode_server_frame(Message::Binary(vec![0xff].into())).is_err());
        let error = parse_server_value(&json!({"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"quota PRIVATE-INPUT-TEXT"}})).unwrap_err().to_string();
        assert!(error.contains("429") && error.contains("配额"));
        assert!(!error.contains("PRIVATE-INPUT-TEXT"));
    }

    #[tokio::test]
    async fn http_rejection_reports_status_without_response_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let result = tokio_tungstenite::accept_hdr_async(tcp, |_request: &tokio_tungstenite::tungstenite::handshake::server::Request, _response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                Err(tokio_tungstenite::tungstenite::http::Response::builder().status(403).body(Some("PRIVATE-API-KEY-123 PRIVATE-BODY".to_owned())).unwrap())
            }).await;
            assert!(result.is_err());
        });
        let mut options = opts(ModelKind::GeneralLive); options.api_key = "PRIVATE-API-KEY-123".into();
        let (trace, captured) = Trace::capturing(&options.api_key, "");
        let (_tx, mut rx) = watch::channel(false);
        let error = connect_with_trace(&format!("ws://{addr}"), &options, &proxy::direct_for_test(), &mut rx, None, &trace).await.err().unwrap().to_string();
        server.await.unwrap();
        let logs = captured.lock().unwrap().join("\n");
        assert!(error.contains("403") && logs.contains("403"));
        assert!(!logs.contains("PRIVATE-BODY") && !error.contains("PRIVATE-BODY"));
        assert!(!logs.contains("PRIVATE-API-KEY-123") && !error.contains("PRIVATE-API-KEY-123"));
    }

    #[test]
    fn classifies_forward_compatible_metadata_without_exposing_payloads() {
        assert_eq!(classify_message(&json!({"toolCallCancellation": {"ids": ["private"]}})), "toolCallCancellation");
        assert_eq!(classify_message(&json!({"futureMetadata": {"private": "value"}})), "协议元数据（已安全忽略）");
    }

    #[test] fn rejects_oversized_credentials_and_invalid_languages() {
        let mut options = opts(ModelKind::GeneralLive);
        options.api_key = "x".repeat(513);
        assert!(options.validate().is_err());
        options.api_key = "test".into();
        options.target_language = "../../secret".into();
        assert!(options.validate().is_err());
    }

    #[test] fn config_fields_are_model_specific() {
        let general = setup(&opts(ModelKind::GeneralLive));
        assert_eq!(general["setup"]["model"], "models/gemini-3.8-live");
        assert!(general["setup"]["generationConfig"].get("thinkingConfig").is_none());
        assert!(general["setup"]["generationConfig"].get("translationConfig").is_none());
        let instruction = general["setup"]["systemInstruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(instruction.contains("Simplified Chinese (zh)") && instruction.contains("Never repeat"));
        assert_eq!(general["setup"]["realtimeInputConfig"]["automaticActivityDetection"]["silenceDurationMs"], 400);
        for (level, expected) in [(ThinkingLevel::Low,"LOW"), (ThinkingLevel::Medium,"MEDIUM"), (ThinkingLevel::High,"HIGH")] {
            let mut options = opts(ModelKind::ExtendedThinking); options.thinking_level = level; options.bidirectional = true;
            assert!(options.validate().is_ok());
            let extended = setup(&options);
            assert_eq!(extended["setup"]["model"], "models/gemini-3.8-live-extended-thinking");
            assert_eq!(extended["setup"]["generationConfig"]["thinkingConfig"]["thinkingLevel"], expected);
            assert!(extended["setup"]["generationConfig"].get("translationConfig").is_none());
            assert!(extended["setup"]["systemInstruction"]["parts"][0]["text"].as_str().unwrap().contains("simultaneous interpreter"));
        }
        assert!(serde_json::from_str::<ThinkingLevel>("\"MINIMAL\"").is_err());
    }
    #[test]
    fn dedicated_35_models_use_distinct_documented_setups() {
        let mut translation = opts(ModelKind::LiveTranslate35);
        let s = setup(&translation);
        assert_eq!(s, json!({"setup": {
            "model": "models/gemini-3.5-live-translate-preview",
            "generationConfig": {"responseModalities": ["AUDIO"],
                "translationConfig": {"targetLanguageCode": "zh-Hans", "echoTargetLanguage": false}},
            "inputAudioTranscription": {}, "outputAudioTranscription": {}
        }}));
        translation.target_language = "ja".into();
        assert_eq!(setup(&translation)["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"], "ja");
        translation.bidirectional = true;
        assert!(translation.validate().is_err());

        let mut transcript = opts(ModelKind::TranscribeLive35);
        assert_eq!(setup(&transcript), json!({"setup": {
            "model": "models/gemini-3.5-transcribe-live",
            "generationConfig": {"responseModalities": ["TEXT"]},
            "inputAudioTranscription": {"languageCodes": [], "mode": "VERBATIM"}
        }}));
        transcript.transcription_mode = TranscriptionMode::Smart;
        assert_eq!(setup(&transcript)["setup"]["inputAudioTranscription"]["mode"], "SMART");
        assert!(transcript.validate().is_ok());
        transcript.bidirectional = true;
        assert!(transcript.validate().is_err());
        assert_eq!(serde_json::from_str::<ModelKind>("\"live_translate\"").unwrap(), ModelKind::ExtendedThinking);
        assert_eq!(serde_json::from_str::<ModelKind>("\"live_translate_35\"").unwrap(), ModelKind::LiveTranslate35);
        assert_eq!(serde_json::from_str::<ModelKind>("\"transcribe_live_35\"").unwrap(), ModelKind::TranscribeLive35);
    }

    #[test]
    fn transcribe_interim_and_final_support_both_json_naming_styles() {
        let interim = parse_server_message(r#"{"serverContent":{"interimInputTranscription":{"text":"hello wor"}}}"#).unwrap();
        assert_eq!(interim.interim_input.unwrap().0, "hello wor");
        assert!(interim.input.is_none());
        let final_message = parse_server_message(r#"{"server_content":{"interim_input_transcription":{"text":"stale"},"input_transcription":{"text":"hello world"},"turn_complete":true}}"#).unwrap();
        assert_eq!(final_message.interim_input.unwrap().0, "stale");
        assert_eq!(final_message.input.unwrap().0, "hello world");
        assert!(final_message.turn_complete);
    }

    #[tokio::test]
    async fn transcribe_mock_websocket_uses_text_setup_and_receives_revisions() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let Message::Text(config) = ws.next().await.unwrap().unwrap() else { panic!("expected setup text") };
            let config: Value = serde_json::from_str(&config).unwrap();
            assert_eq!(config["setup"]["model"], "models/gemini-3.5-transcribe-live");
            assert_eq!(config["setup"]["generationConfig"]["responseModalities"], json!(["TEXT"]));
            assert!(config["setup"].get("outputAudioTranscription").is_none());
            ws.send(Message::Binary(br#"{"setupComplete":{}}"#.to_vec().into())).await.unwrap();
            let audio = ws.next().await.unwrap().unwrap();
            assert!(audio.is_text());
            for response in [
                r#"{"serverContent":{"interimInputTranscription":{"text":"hello wor"}}}"#,
                r#"{"server_content":{"interim_input_transcription":{"text":"hello world"}}}"#,
                r#"{"serverContent":{"inputTranscription":{"text":"Hello, world."}}}"#,
                r#"{"serverContent":{"interimInputTranscription":{"text":"another"}}}"#,
                r#"{"server_content":{"input_transcription":{"text":"Another sentence."}}}"#,
            ] { ws.send(Message::Text(response.into())).await.unwrap(); }
        });
        let options = opts(ModelKind::TranscribeLive35);
        let (trace, _) = Trace::capturing(&options.api_key, "");
        let (_tx, mut rx) = watch::channel(false);
        let mut socket = tokio::time::timeout(Duration::from_secs(3), connect_with_trace(&format!("ws://{addr}"), &options, &proxy::direct_for_test(), &mut rx, None, &trace)).await.unwrap().unwrap();
        socket.send(audio_frame(&[0, 0, 1, 0])).await.unwrap();
        let mut caption = String::new();
        for expected in ["hello wor", "hello world", "Hello, world.", "another", "Another sentence."] {
            let ServerFrame::Data { value, .. } = decode_server_frame(socket.next().await.unwrap().unwrap()).unwrap() else { panic!("expected data") };
            let p = parse_server_value(&value).unwrap();
            crate::replace_transcript(&mut caption, p.interim_input.as_ref().map(|x| x.0.as_str()), p.input.as_ref().map(|x| x.0.as_str()));
            assert_eq!(caption, expected);
        }
        server.await.unwrap();
    }

    #[test] fn extended_status_may_be_nested_or_top_level() {
        let nested = parse_server_message(r#"{"serverContent":{"turnComplete":true,"interactionStatus":"IN_PROGRESS"}}"#).unwrap();
        assert!(nested.turn_complete);
        assert_eq!(nested.interaction_status, Some(InteractionStatus::InProgress));
        let top = parse_server_message(r#"{"interaction_status":"IDLE"}"#).unwrap();
        assert_eq!(top.interaction_status, Some(InteractionStatus::Idle));
        assert_eq!(serde_json::from_str::<ModelKind>("\"live_translate\"").unwrap(), ModelKind::ExtendedThinking);
    }
    #[tokio::test] async fn extended_mock_websocket_continues_after_an_intermediate_turn() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let setup = ws.next().await.unwrap().unwrap().into_text().unwrap();
            let setup: Value = serde_json::from_str(&setup).unwrap();
            assert_eq!(setup["setup"]["model"], "models/gemini-3.8-live-extended-thinking");
            assert_eq!(setup["setup"]["generationConfig"]["thinkingConfig"]["thinkingLevel"], "HIGH");
            ws.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            ws.send(Message::Text(r#"{"serverContent":{"turnComplete":true,"interactionStatus":"IN_PROGRESS","outputTranscription":{"text":"working"}}}"#.into())).await.unwrap();
            ws.send(Message::Text(r#"{"serverContent":{"turnComplete":true,"interactionStatus":"IDLE","outputTranscription":{"text":"done"},"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AQI="}}]}}}"#.into())).await.unwrap();
        });
        let mut options = opts(ModelKind::ExtendedThinking);
        options.thinking_level = ThinkingLevel::High; options.bidirectional = true;
        let (_tx, mut rx) = watch::channel(false);
        let mut client = connect_to(&format!("ws://{addr}"), &options, &proxy::direct_for_test(), &mut rx).await.unwrap();
        let intermediate = parse_server_message(&client.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert!(intermediate.turn_complete);
        assert_eq!(intermediate.interaction_status, Some(InteractionStatus::InProgress));
        assert_eq!(intermediate.output.unwrap().0, "working");
        let final_turn = parse_server_message(&client.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(final_turn.interaction_status, Some(InteractionStatus::Idle));
        assert_eq!(final_turn.output.unwrap().0, "done");
        assert_eq!(final_turn.audio, vec![vec![1, 2]]);
        server.await.unwrap();
    }
    #[test] fn parses_all_audio_parts_and_transcripts() {
        let message = json!({"serverContent": {"inputTranscription":{"text":"hello"},"outputTranscription":{"text":"你好"}, "modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AQI="}},{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AwQ="}}]}}}).to_string();
        let p = parse_server_message(&message).unwrap();
        assert_eq!(p.input.unwrap().0, "hello"); assert_eq!(p.output.unwrap().0,"你好");
        assert_eq!(p.audio, vec![vec![1,2],vec![3,4]]);
    }
    #[test]
    fn rejects_oversized_transcripts_audio_and_frames() {
        let transcript = "x".repeat(MAX_TRANSCRIPT_BYTES + 1);
        let value = json!({"serverContent":{"inputTranscription":{"text": transcript}}});
        assert!(parse_server_value(&value).unwrap_err().to_string().contains("转写文本超过"));

        let encoded = "A".repeat((MAX_AUDIO_BYTES_PER_MESSAGE / 3 + 1) * 4 + 1);
        let value = json!({"serverContent":{"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":encoded}}]}}});
        assert!(parse_server_value(&value).unwrap_err().to_string().contains("音频消息超过"));

        let oversized = format!("{{\"padding\":\"{}\"}}", "x".repeat(MAX_SERVER_MESSAGE_BYTES));
        assert!(decode_server_frame(Message::Text(oversized.into())).is_err());
    }

    #[test] fn audio_frame_is_pcm_base64_and_stream_end_is_explicit() {
        let frame = audio_frame(&[1, 2, 3, 4]).into_text().unwrap();
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["realtimeInput"]["audio"]["mimeType"], "audio/pcm;rate=16000");
        assert_eq!(STANDARD.decode(v["realtimeInput"]["audio"]["data"].as_str().unwrap()).unwrap(), vec![1,2,3,4]);
        let end: Value = serde_json::from_str(&stream_end().into_text().unwrap()).unwrap();
        assert_eq!(end["realtimeInput"]["audioStreamEnd"], true);
    }
    #[tokio::test] async fn waits_for_setup_complete_on_real_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(tcp, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                assert_eq!(request.headers().get("x-goog-api-key").unwrap(), "test");
                assert!(request.uri().query().is_none());
                Ok(response)
            }).await.unwrap();
            let setup = ws.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(setup.contains("gemini-3.8-live"));
            // The client must not send audio until setupComplete arrives.
            assert!(tokio::time::timeout(Duration::from_millis(30), ws.next()).await.is_err());
            ws.send(Message::Text("{\"setupComplete\":{}}".into())).await.unwrap();
        });
        let (_tx, mut rx) = watch::channel(false);
        let _client = connect_to(&format!("ws://{addr}"), &opts(ModelKind::GeneralLive), &proxy::direct_for_test(), &mut rx).await.unwrap();
        server.await.unwrap();
    }
    #[tokio::test] async fn cancellation_interrupts_wait_for_setup() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let _ = ws.next().await.unwrap().unwrap();
            let _ = ready_tx.send(());
            let _ = ws.next().await;
        });
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let client = tokio::spawn(async move { connect_to(&format!("ws://{addr}"), &opts(ModelKind::GeneralLive), &proxy::direct_for_test(), &mut cancel_rx).await });
        ready_rx.await.unwrap(); cancel_tx.send(true).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), client).await.unwrap().unwrap().unwrap_err();
        assert!(error.to_string().contains("已停止"));
        server.await.unwrap();
    }

    // Local CONNECT proxy and TLS WebSocket server: no real Google key or
    // Internet is needed, and the proxy must never see the API key in clear.
    #[tokio::test]
    async fn verified_wss_through_http_connect_preserves_credentials_boundaries() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use std::sync::Arc;
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_der = cert.der().clone();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into();
        let server_config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert_der.clone()], key).unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let mut roots = rustls::RootCertStore::empty(); roots.add(cert_der).unwrap();
        let client_config = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let tls = acceptor.accept(tcp).await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(tls, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                assert_eq!(request.headers().get("x-goog-api-key").unwrap(), "TEST-API-KEY-PRIVATE-123456789");
                assert!(request.headers().get("proxy-authorization").is_none());
                Ok(response)
            }).await.unwrap();
            let setup = ws.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(setup.contains("gemini-3.8-live"));
            ws.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            let _ = ws.close(None).await;
        });
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let proxy_task = tokio::spawn(async move {
            let (mut incoming, _) = proxy_listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") { header.push(incoming.read_u8().await.unwrap()); }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with(&format!("CONNECT localhost:{} HTTP/1.1\r\n", server_addr.port())));
            assert!(header.contains(&format!("Proxy-Authorization: Basic {}", STANDARD.encode(b"alice:local-password"))));
            assert!(!header.contains("TEST-API-KEY-PRIVATE"));
            incoming.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
            let mut target = TcpStream::connect(server_addr).await.unwrap();
            // Forward and inspect encrypted client-to-server bytes. Never log them.
            let (mut client_read, mut client_write) = incoming.split();
            let (mut target_read, mut target_write) = target.split();
            let c2s = async {
                let mut captured = Vec::new(); let mut buf = [0u8; 4096];
                loop {
                    let n = client_read.read(&mut buf).await.unwrap_or(0);
                    if n == 0 { let _ = target_write.shutdown().await; break; }
                    captured.extend_from_slice(&buf[..n]);
                    if target_write.write_all(&buf[..n]).await.is_err() { break; }
                }
                assert!(!captured.windows(b"TEST-API-KEY-PRIVATE-123456789".len()).any(|w| w == b"TEST-API-KEY-PRIVATE-123456789"));
            };
            let s2c = async { let _ = tokio::io::copy(&mut target_read, &mut client_write).await; let _ = client_write.shutdown().await; };
            tokio::join!(c2s, s2c);
        });
        let mut options = opts(ModelKind::GeneralLive); options.api_key = "TEST-API-KEY-PRIVATE-123456789".into();
        let route = proxy::manual_for_test(&format!("http://alice:local-password@{proxy_addr}")).unwrap();
        let (_tx, mut rx) = watch::channel(false);
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(client_config));
        let socket = connect_using(&format!("wss://localhost:{}/", server_addr.port()), &options, &route, &mut rx, Some(connector)).await.unwrap();
        drop(socket);
        tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(3), proxy_task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn default_wss_rejects_untrusted_certificate_through_proxy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use std::sync::Arc;
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into();
        let config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert.der().clone()], key).unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let server_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server_listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = server_listener.accept().await.unwrap();
            assert!(acceptor.accept(tcp).await.is_err()); // no WebSocket/API key
        });
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let forwarder = tokio::spawn(async move {
            let (mut client, _) = proxy_listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") { header.push(client.read_u8().await.unwrap()); }
            assert!(!String::from_utf8_lossy(&header).contains("x-goog-api-key"));
            client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            let mut target = TcpStream::connect(server_addr).await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
        });
        let route = proxy::manual_for_test(&format!("http://alice:secret@{proxy_addr}")).unwrap();
        let (_tx, mut rx) = watch::channel(false);
        let error = connect_to(&format!("wss://localhost:{}/", server_addr.port()), &opts(ModelKind::GeneralLive), &route, &mut rx).await.err().unwrap();
        let text = format!("{error:#}");
        assert!(text.contains("TLS") || text.contains("certificate") || text.contains("证书"));
        assert!(!text.contains("alice") && !text.contains("secret"));
        tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(3), forwarder).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_stalled_proxy_handshake() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let proxy_task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") { header.push(client.read_u8().await.unwrap()); }
            let _ = ready_tx.send(());
            let mut buf = [0; 1]; let _ = client.read(&mut buf).await; // wait until cancelled socket closes
        });
        let route = proxy::manual_for_test(&format!("http://{proxy_addr}")).unwrap();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let task = tokio::spawn(async move { connect_to("wss://example.invalid/", &opts(ModelKind::GeneralLive), &route, &mut cancel_rx).await });
        ready_rx.await.unwrap(); cancel_tx.send(true).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().err().unwrap();
        assert!(error.to_string().contains("已停止"));
        tokio::time::timeout(Duration::from_secs(2), proxy_task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn websocket_through_socks5_proxy_sends_setup_after_handshake() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = target_listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = target_listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(tcp, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                assert_eq!(request.headers().get("x-goog-api-key").unwrap(), "test"); Ok(response)
            }).await.unwrap();
            let setup = ws.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(setup.contains("gemini-3.8-live"));
            ws.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            let _ = ws.close(None).await;
        });
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let forwarder = tokio::spawn(async move {
            let (mut client, _) = proxy_listener.accept().await.unwrap();
            let mut greeting = [0; 3]; client.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]); client.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 10]; client.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 1]);
            assert_eq!(&request[4..8], &[127, 0, 0, 1]);
            assert_eq!(&request[8..], &target.port().to_be_bytes());
            client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await.unwrap();
            let mut server = TcpStream::connect(target).await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
        });
        let route = proxy::manual_for_test(&format!("socks5://{proxy_addr}")).unwrap();
        let (_tx, mut rx) = watch::channel(false);
        let client = connect_to(&format!("ws://{target}"), &opts(ModelKind::GeneralLive), &route, &mut rx).await.unwrap();
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(3), forwarder).await.unwrap().unwrap();
    }

}
