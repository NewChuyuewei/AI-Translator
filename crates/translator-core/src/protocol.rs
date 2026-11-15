//! Pure Gemini Live wire-message construction.
//!
//! This module has no socket, UI, runtime, or API-key dependency. Keeping wire
//! construction here permits deterministic verification with an in-memory
//! adapter and prevents credentials from entering payloads.

use crate::{ModelKind, SessionOptions, ThinkingLevel, TranscriptionMode};
use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub const PCM_SAMPLE_RATE_HZ: u32 = 16_000;
pub const MAX_AUDIO_BYTES_PER_MESSAGE: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveProtocolConfig {
    pub model: ModelKind,
    pub thinking_level: ThinkingLevel,
    pub transcription_mode: TranscriptionMode,
    pub target_language: String,
    pub bidirectional: bool,
    pub other_language: String,
}

impl From<&SessionOptions> for LiveProtocolConfig {
    fn from(options: &SessionOptions) -> Self {
        Self {
            model: options.model,
            thinking_level: options.thinking_level,
            transcription_mode: options.transcription_mode,
            target_language: options.target_language.clone(),
            bidirectional: options.bidirectional,
            other_language: options.other_language.clone(),
        }
    }
}

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

/// SWR-PROTO-001, SWR-PROTO-002: construct the model-specific setup without
/// accepting or serializing credentials.
pub fn setup(config: &LiveProtocolConfig) -> Value {
    match config.model {
        ModelKind::LiveTranslate35 => {
            let language = if config.target_language.eq_ignore_ascii_case("zh") {
                "zh-Hans"
            } else {
                &config.target_language
            };
            json!({"setup": {
                "model": "models/gemini-3.5-live-translate-preview",
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "translationConfig": {"targetLanguageCode": language, "echoTargetLanguage": false}
                },
                "inputAudioTranscription": {}, "outputAudioTranscription": {}
            }})
        }
        ModelKind::TranscribeLive35 => json!({"setup": {
            "model": "models/gemini-3.5-transcribe-live",
            "generationConfig": {"responseModalities": ["TEXT"]},
            "inputAudioTranscription": {"languageCodes": [], "mode": config.transcription_mode}
        }}),
        ModelKind::GeneralLive | ModelKind::ExtendedThinking => {
            let target = language_instruction_name(&config.target_language);
            let other = language_instruction_name(&config.other_language);
            let instruction = if config.bidirectional {
                format!(
                    "Act only as a low-latency simultaneous interpreter. Translate speech in {target} into {other}, and speech in {other} into {target}. Start each translation as soon as a meaningful phrase is available. Output only the translation in the opposite language. Never repeat or transcribe the source language, answer questions, explain, add commentary, or mix source and target languages."
                )
            } else {
                format!(
                    "Act only as a low-latency simultaneous interpreter. Translate every heard utterance into {target}. Start speaking the translation as soon as a meaningful phrase is available instead of waiting for a full paragraph. Output only {target}. Never repeat or transcribe the source language, answer questions, explain, add commentary, or mix source and target languages."
                )
            };
            let (model, generation) = if config.model == ModelKind::GeneralLive {
                (
                    "models/gemini-3.8-live",
                    json!({"responseModalities": ["AUDIO"]}),
                )
            } else {
                (
                    "models/gemini-3.8-live-extended-thinking",
                    json!({
                        "responseModalities": ["AUDIO"],
                        "thinkingConfig": {"thinkingLevel": config.thinking_level}
                    }),
                )
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

/// SWR-PROTO-003: reject oversized input before Base64/JSON allocation.
pub fn audio_frame(data: &[u8]) -> Result<String> {
    if data.len() > MAX_AUDIO_BYTES_PER_MESSAGE {
        return Err(anyhow!("音频帧超过协议大小限制"));
    }
    Ok(json!({"realtimeInput": {"audio": {
        "mimeType": format!("audio/pcm;rate={PCM_SAMPLE_RATE_HZ}"),
        "data": STANDARD.encode(data)
    }}})
    .to_string())
}

pub fn stream_end() -> String {
    json!({"realtimeInput": {"audioStreamEnd": true}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(model: ModelKind) -> LiveProtocolConfig {
        LiveProtocolConfig {
            model,
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            target_language: "zh".into(),
            bidirectional: false,
            other_language: "en".into(),
        }
    }

    #[test]
    fn swr_proto_001_setup_is_model_specific_and_contains_no_credentials() {
        let models = [
            (ModelKind::GeneralLive, "models/gemini-3.8-live"),
            (
                ModelKind::ExtendedThinking,
                "models/gemini-3.8-live-extended-thinking",
            ),
            (
                ModelKind::LiveTranslate35,
                "models/gemini-3.5-live-translate-preview",
            ),
            (
                ModelKind::TranscribeLive35,
                "models/gemini-3.5-transcribe-live",
            ),
        ];
        for (model, expected) in models {
            let value = setup(&config(model));
            assert_eq!(value["setup"]["model"], expected);
            let encoded = value.to_string();
            assert!(!encoded.contains("apiKey"));
            assert!(!encoded.contains("PRIVATE"));
        }
    }

    #[test]
    fn swr_proto_002_translation_and_transcription_semantics_are_distinct() {
        let translation = setup(&config(ModelKind::LiveTranslate35));
        assert_eq!(
            translation["setup"]["generationConfig"]["responseModalities"],
            json!(["AUDIO"])
        );
        assert_eq!(
            translation["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
            "zh-Hans"
        );
        let transcription = setup(&config(ModelKind::TranscribeLive35));
        assert_eq!(
            transcription["setup"]["generationConfig"]["responseModalities"],
            json!(["TEXT"])
        );
        assert!(
            transcription["setup"]
                .get("outputAudioTranscription")
                .is_none()
        );
    }

    #[test]
    fn swr_proto_003_audio_frames_are_bounded_and_exactly_encoded() {
        let encoded = audio_frame(&[0, 1, 2, 3]).expect("bounded fixture must encode");
        let value: Value = serde_json::from_str(&encoded).expect("encoder must produce JSON");
        assert_eq!(
            value["realtimeInput"]["audio"]["mimeType"],
            "audio/pcm;rate=16000"
        );
        assert_eq!(value["realtimeInput"]["audio"]["data"], "AAECAw==");
        assert!(audio_frame(&vec![0; MAX_AUDIO_BYTES_PER_MESSAGE + 1]).is_err());
    }

    #[test]
    fn swr_proto_004_stream_end_has_no_implicit_audio_payload() {
        let value: Value = serde_json::from_str(&stream_end()).expect("encoder must produce JSON");
        assert_eq!(value, json!({"realtimeInput": {"audioStreamEnd": true}}));
    }
}
