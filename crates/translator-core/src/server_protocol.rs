//! Bounded, credential-safe Gemini server-frame decoding.
//!
//! WebSocket control handling remains in platform transport adapters. This
//! module accepts only text/binary payload bytes and returns typed state.

use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub const MAX_SERVER_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;
pub const MAX_AUDIO_BYTES_PER_MESSAGE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFrameKind {
    Text,
    Binary,
}

impl DataFrameKind {
    pub fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Binary => "二进制",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionStatus {
    InProgress,
    Idle,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ParsedMessage {
    pub input: Option<(String, bool)>,
    pub interim_input: Option<(String, bool)>,
    pub output: Option<(String, bool)>,
    pub audio: Vec<Vec<u8>>,
    pub interrupted: bool,
    pub turn_complete: bool,
    pub go_away: bool,
    pub interaction_status: Option<InteractionStatus>,
}

fn field<'a>(value: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    value.get(camel).or_else(|| value.get(snake))
}

/// SWR-RX-001: decode bounded UTF-8 JSON without reflecting payload content.
pub fn decode_data_frame(kind: DataFrameKind, bytes: &[u8]) -> Result<Value> {
    if bytes.len() > MAX_SERVER_MESSAGE_BYTES {
        return Err(anyhow!("Gemini {}帧超过大小限制", kind.diagnostic_label()));
    }
    let value: Value = match kind {
        DataFrameKind::Text => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| anyhow!("Gemini 文本帧不是有效 UTF-8"))?;
            serde_json::from_str(text).map_err(|error| {
                anyhow!(
                    "Gemini 文本帧不是有效 JSON（行 {}，列 {}）",
                    error.line(),
                    error.column()
                )
            })?
        }
        DataFrameKind::Binary => {
            std::str::from_utf8(bytes).map_err(|_| anyhow!("Gemini 二进制帧不是有效 UTF-8"))?;
            serde_json::from_slice(bytes).map_err(|error| {
                anyhow!(
                    "Gemini 二进制帧不是有效 JSON（行 {}，列 {}）",
                    error.line(),
                    error.column()
                )
            })?
        }
    };
    if !value.is_object() {
        return Err(anyhow!(
            "Gemini {}帧的 JSON 顶层不是对象",
            kind.diagnostic_label()
        ));
    }
    Ok(value)
}

/// Return only fixed labels; untrusted keys and values never enter diagnostics.
pub fn classify_message(value: &Value) -> &'static str {
    if value.get("error").is_some() {
        "error"
    } else if field(value, "setupComplete", "setup_complete").is_some() {
        "setupComplete"
    } else if field(value, "serverContent", "server_content").is_some() {
        "serverContent"
    } else if field(value, "goAway", "go_away").is_some() {
        "goAway"
    } else if field(value, "usageMetadata", "usage_metadata").is_some() {
        "usageMetadata"
    } else if field(
        value,
        "sessionResumptionUpdate",
        "session_resumption_update",
    )
    .is_some()
    {
        "sessionResumptionUpdate"
    } else if field(value, "interactionStatus", "interaction_status").is_some() {
        "interactionStatus"
    } else if field(value, "toolCall", "tool_call").is_some() {
        "toolCall"
    } else if field(value, "toolCallCancellation", "tool_call_cancellation").is_some() {
        "toolCallCancellation"
    } else {
        "协议元数据（已安全忽略）"
    }
}

/// SWR-RX-004: map untrusted text to an allowlisted hint without echoing it.
pub fn safe_error_hint(raw: &str) -> Option<&'static str> {
    let normalized = raw
        .chars()
        .take(512)
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.contains("api key")
        || normalized.contains("api_key")
        || normalized.contains("unauthenticated")
        || normalized.contains("permission_denied")
        || normalized.contains("permission denied")
    {
        Some("请检查 API Key 和账户权限")
    } else if normalized.contains("quota")
        || normalized.contains("resource_exhausted")
        || normalized.contains("rate limit")
    {
        Some("请检查配额和调用频率")
    } else if normalized.contains("not found")
        || normalized.contains("not_found")
        || normalized.contains("not supported")
        || normalized.contains("not available")
        || normalized.contains("unsupported model")
    {
        Some("请检查模型 ID、API 版本及账户可用性")
    } else if normalized.contains("region")
        || normalized.contains("location")
        || normalized.contains("country")
    {
        Some("请检查 Gemini 服务的地区可用性")
    } else if normalized.contains("invalid argument") || normalized.contains("invalid_argument") {
        Some("setup 参数可能不受支持")
    } else {
        None
    }
}

fn status_label(raw: &str) -> Option<&'static str> {
    match raw {
        "CANCELLED" => Some("CANCELLED"),
        "UNKNOWN" => Some("UNKNOWN"),
        "INVALID_ARGUMENT" => Some("INVALID_ARGUMENT"),
        "DEADLINE_EXCEEDED" => Some("DEADLINE_EXCEEDED"),
        "NOT_FOUND" => Some("NOT_FOUND"),
        "ALREADY_EXISTS" => Some("ALREADY_EXISTS"),
        "PERMISSION_DENIED" => Some("PERMISSION_DENIED"),
        "RESOURCE_EXHAUSTED" => Some("RESOURCE_EXHAUSTED"),
        "FAILED_PRECONDITION" => Some("FAILED_PRECONDITION"),
        "ABORTED" => Some("ABORTED"),
        "OUT_OF_RANGE" => Some("OUT_OF_RANGE"),
        "UNIMPLEMENTED" => Some("UNIMPLEMENTED"),
        "INTERNAL" => Some("INTERNAL"),
        "UNAVAILABLE" => Some("UNAVAILABLE"),
        "DATA_LOSS" => Some("DATA_LOSS"),
        "UNAUTHENTICATED" => Some("UNAUTHENTICATED"),
        _ => None,
    }
}

pub fn close_description(code: Option<u16>, hint: Option<&'static str>) -> String {
    let code = code.map_or_else(|| "未提供".into(), |value| value.to_string());
    let suffix = hint.map_or_else(String::new, |value| format!("；{value}"));
    format!("Gemini 关闭 WebSocket（关闭码 {code}）{suffix}；原始关闭原因未记录")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerErrorContext {
    Setup,
    Session,
}

pub fn server_error(error: &Value, context: ServerErrorContext) -> anyhow::Error {
    let prefix = match context {
        ServerErrorContext::Setup => "Gemini setup 被拒绝",
        ServerErrorContext::Session => "Gemini 返回错误",
    };
    let code = error
        .get("code")
        .and_then(Value::as_u64)
        .filter(|value| (100..=599).contains(value));
    let status = error
        .get("status")
        .and_then(Value::as_str)
        .and_then(status_label);
    let hint = error
        .get("message")
        .and_then(Value::as_str)
        .and_then(safe_error_hint)
        .or_else(|| status.and_then(safe_error_hint));
    let mut details = Vec::with_capacity(3);
    if let Some(code) = code {
        details.push(format!("代码 {code}"));
    }
    if let Some(status) = status {
        details.push(format!("状态 {status}"));
    }
    if let Some(hint) = hint {
        details.push(hint.into());
    }
    if details.is_empty() {
        details.push("未提供可安全显示的错误代码".into());
    }
    anyhow!("{prefix}（{}）；原始响应未记录", details.join("；"))
}

fn interaction_status(value: Option<&Value>) -> Option<InteractionStatus> {
    match value?.as_str()? {
        "IN_PROGRESS" => Some(InteractionStatus::InProgress),
        "IDLE" => Some(InteractionStatus::Idle),
        _ => None,
    }
}

fn transcription(value: Option<&Value>) -> Result<Option<(String, bool)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.get("text").and_then(Value::as_str) else {
        return Ok(None);
    };
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(anyhow!("Gemini 转写文本超过大小限制"));
    }
    let finished = value
        .get("finished")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Some((text.to_owned(), finished)))
}

/// SWR-RX-002, SWR-RX-003: parse bounded transcript/audio state and accept
/// documented camelCase plus forward-compatible snake_case envelopes.
pub fn parse_server_value(root: &Value) -> Result<ParsedMessage> {
    if let Some(error) = root.get("error") {
        return Err(server_error(error, ServerErrorContext::Session));
    }
    let mut parsed = ParsedMessage {
        go_away: field(root, "goAway", "go_away").is_some(),
        interaction_status: interaction_status(field(
            root,
            "interactionStatus",
            "interaction_status",
        )),
        ..Default::default()
    };
    if let Some(content) = field(root, "serverContent", "server_content") {
        parsed.interaction_status =
            interaction_status(field(content, "interactionStatus", "interaction_status"))
                .or(parsed.interaction_status);
        parsed.input = transcription(field(content, "inputTranscription", "input_transcription"))?;
        parsed.interim_input = transcription(field(
            content,
            "interimInputTranscription",
            "interim_input_transcription",
        ))?;
        parsed.output = transcription(field(
            content,
            "outputTranscription",
            "output_transcription",
        ))?;
        parsed.interrupted = content
            .get("interrupted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        parsed.turn_complete = field(content, "turnComplete", "turn_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(parts) = field(content, "modelTurn", "model_turn")
            .and_then(|value| value.get("parts"))
            .and_then(Value::as_array)
        {
            let mut audio_bytes = 0usize;
            for part in parts {
                let Some(inline) = field(part, "inlineData", "inline_data") else {
                    continue;
                };
                let Some(encoded) = inline.get("data").and_then(Value::as_str) else {
                    continue;
                };
                let mime = field(inline, "mimeType", "mime_type")
                    .and_then(Value::as_str)
                    .unwrap_or("audio/pcm;rate=24000");
                if !mime.starts_with("audio/pcm") {
                    continue;
                }
                if mime.contains("rate=") && !mime.contains("rate=24000") {
                    return Err(anyhow!("不支持的 Gemini 输出采样率（需要 24000 Hz）"));
                }
                if encoded.len() > (MAX_AUDIO_BYTES_PER_MESSAGE / 3 + 1) * 4 {
                    return Err(anyhow!("Gemini 音频消息超过大小限制"));
                }
                let decoded = STANDARD
                    .decode(encoded)
                    .map_err(|_| anyhow!("Gemini 音频 Base64 无效"))?;
                audio_bytes = audio_bytes
                    .checked_add(decoded.len())
                    .ok_or_else(|| anyhow!("Gemini 音频消息超过大小限制"))?;
                if audio_bytes > MAX_AUDIO_BYTES_PER_MESSAGE {
                    return Err(anyhow!("Gemini 音频消息超过大小限制"));
                }
                parsed.audio.push(decoded);
            }
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn swr_rx_001_text_and_binary_decoding_is_bounded_and_content_safe() {
        let fixture = br#"{"serverContent":{}}"#;
        let text = decode_data_frame(DataFrameKind::Text, fixture).expect("text fixture");
        let binary = decode_data_frame(DataFrameKind::Binary, fixture).expect("binary fixture");
        assert_eq!(text, binary);
        let secret = "PRIVATE-SERVER-CONTENT";
        let error = decode_data_frame(DataFrameKind::Text, secret.as_bytes())
            .expect_err("invalid JSON must fail")
            .to_string();
        assert!(!error.contains(secret));
        assert!(decode_data_frame(DataFrameKind::Binary, &[0xff]).is_err());
        assert!(
            decode_data_frame(
                DataFrameKind::Text,
                &vec![b'x'; MAX_SERVER_MESSAGE_BYTES + 1]
            )
            .is_err()
        );
    }

    #[test]
    fn swr_rx_002_parses_naming_styles_and_typed_state() {
        let value = json!({"server_content": {
            "interim_input_transcription": {"text": "hello"},
            "output_transcription": {"text": "你好", "finished": true},
            "interaction_status": "IN_PROGRESS",
            "turn_complete": true
        }});
        let parsed = parse_server_value(&value).expect("bounded fixture must parse");
        assert_eq!(parsed.interim_input, Some(("hello".into(), false)));
        assert_eq!(parsed.output, Some(("你好".into(), true)));
        assert_eq!(
            parsed.interaction_status,
            Some(InteractionStatus::InProgress)
        );
        assert!(parsed.turn_complete);
    }

    #[test]
    fn swr_rx_003_audio_and_transcripts_are_bounded_before_growth() {
        let audio = json!({"serverContent": {"modelTurn": {"parts": [{"inlineData": {
            "mimeType": "audio/pcm;rate=24000", "data": "AQIDBA=="
        }}]}}});
        assert_eq!(
            parse_server_value(&audio).expect("valid audio").audio,
            vec![vec![1, 2, 3, 4]]
        );
        let oversized_text = "x".repeat(MAX_TRANSCRIPT_BYTES + 1);
        assert!(
            parse_server_value(&json!({"serverContent": {
                "inputTranscription": {"text": oversized_text}
            }}))
            .is_err()
        );
        let oversized_audio = "A".repeat((MAX_AUDIO_BYTES_PER_MESSAGE / 3 + 1) * 4 + 1);
        assert!(
            parse_server_value(&json!({"serverContent": {"modelTurn": {"parts": [{
                "inlineData": {"data": oversized_audio}
            }]}}}))
            .is_err()
        );
    }

    #[test]
    fn swr_rx_004_server_errors_are_allowlisted_and_redacted() {
        let secret = "PRIVATE-TRANSCRIPT-AND-KEY";
        let error = parse_server_value(&json!({"error": {
            "code": 429, "status": "RESOURCE_EXHAUSTED", "message": format!("quota {secret}")
        }}))
        .expect_err("server error must fail")
        .to_string();
        assert!(error.contains("429"));
        assert!(error.contains("RESOURCE_EXHAUSTED"));
        assert!(error.contains("配额"));
        assert!(!error.contains(secret));
        assert_eq!(
            safe_error_hint(&format!("api key {secret}")),
            Some("请检查 API Key 和账户权限")
        );
    }
}
