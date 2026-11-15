//! Transport-neutral Gemini session sequencing for deterministic tests and UI
//! adapters. Socket implementations only move bounded bytes to/from this type.

use crate::{
    protocol::{self, LiveProtocolConfig},
    server_protocol::{
        DataFrameKind, ParsedMessage, ServerErrorContext, classify_message, decode_data_frame,
        parse_server_value, server_error,
    },
};
use anyhow::{Result, anyhow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverState {
    AwaitingSetup,
    Streaming,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DriverEvent {
    SetupComplete,
    Message(ParsedMessage),
    Metadata(&'static str),
}

pub struct SessionDriver {
    state: DriverState,
    setup_message: String,
}

impl SessionDriver {
    /// SWR-SESSION-001: initialization produces one deterministic setup and no
    /// audio; credentials cannot enter because `LiveProtocolConfig` has none.
    pub fn new(config: &LiveProtocolConfig) -> Self {
        Self {
            state: DriverState::AwaitingSetup,
            setup_message: protocol::setup(config).to_string(),
        }
    }

    pub fn state(&self) -> DriverState {
        self.state
    }

    pub fn setup_message(&self) -> &str {
        &self.setup_message
    }

    /// SWR-SESSION-002: only setupComplete enables outbound PCM.
    pub fn receive(&mut self, kind: DataFrameKind, bytes: &[u8]) -> Result<DriverEvent> {
        if self.state == DriverState::Closed {
            return Err(anyhow!("会话已关闭，不能处理服务端消息"));
        }
        let value = decode_data_frame(kind, bytes)?;
        if let Some(error) = value.get("error") {
            let context = if self.state == DriverState::AwaitingSetup {
                ServerErrorContext::Setup
            } else {
                ServerErrorContext::Session
            };
            return Err(server_error(error, context));
        }
        let setup_complete =
            value.get("setupComplete").is_some() || value.get("setup_complete").is_some();
        if setup_complete {
            if self.state != DriverState::AwaitingSetup {
                return Err(anyhow!("收到重复的 setupComplete"));
            }
            self.state = DriverState::Streaming;
            return Ok(DriverEvent::SetupComplete);
        }
        let category = classify_message(&value);
        let parsed = parse_server_value(&value)?;
        let has_session_data = parsed.input.is_some()
            || parsed.interim_input.is_some()
            || parsed.output.is_some()
            || !parsed.audio.is_empty()
            || parsed.interrupted
            || parsed.turn_complete
            || parsed.go_away
            || parsed.interaction_status.is_some();
        if has_session_data {
            if self.state != DriverState::Streaming {
                return Err(anyhow!("setupComplete 前收到会话数据"));
            }
            Ok(DriverEvent::Message(parsed))
        } else {
            Ok(DriverEvent::Metadata(category))
        }
    }

    pub fn encode_audio(&self, pcm: &[u8]) -> Result<String> {
        if self.state != DriverState::Streaming {
            return Err(anyhow!("setupComplete 前不能发送音频"));
        }
        protocol::audio_frame(pcm)
    }

    /// SWR-SESSION-003: close exactly once and never append residual PCM.
    pub fn close(&mut self) -> Result<String> {
        if self.state == DriverState::Closed {
            return Err(anyhow!("会话已经关闭"));
        }
        self.state = DriverState::Closed;
        Ok(protocol::stream_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelKind, ThinkingLevel, TranscriptionMode};
    use serde_json::Value;

    fn config() -> LiveProtocolConfig {
        LiveProtocolConfig {
            model: ModelKind::GeneralLive,
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            target_language: "zh".into(),
            bidirectional: false,
            other_language: "en".into(),
        }
    }

    #[test]
    fn swr_session_001_memory_session_runs_without_ui_or_socket() {
        let mut driver = SessionDriver::new(&config());
        assert_eq!(driver.state(), DriverState::AwaitingSetup);
        let setup: Value = serde_json::from_str(driver.setup_message()).expect("setup JSON");
        assert_eq!(setup["setup"]["model"], "models/gemini-3.8-live");
        assert!(driver.encode_audio(&[0, 1]).is_err());
        assert_eq!(
            driver
                .receive(DataFrameKind::Binary, br#"{"setupComplete":{}}"#)
                .expect("setup completion"),
            DriverEvent::SetupComplete
        );
        assert_eq!(driver.state(), DriverState::Streaming);
        let message = driver.encode_audio(&[0, 1]).expect("streaming audio");
        assert!(message.contains("AAE="));
        let event = driver
            .receive(
                DataFrameKind::Text,
                br#"{"serverContent":{"outputTranscription":{"text":"translated"}}}"#,
            )
            .expect("bounded response");
        let DriverEvent::Message(parsed) = event else {
            panic!("expected parsed message");
        };
        assert_eq!(parsed.output, Some(("translated".into(), false)));
        let end = driver.close().expect("first close");
        assert!(end.contains("audioStreamEnd"));
        assert_eq!(driver.state(), DriverState::Closed);
    }

    #[test]
    fn swr_session_002_rejects_out_of_order_and_duplicate_events() {
        let mut driver = SessionDriver::new(&config());
        assert!(
            driver
                .receive(
                    DataFrameKind::Text,
                    br#"{"serverContent":{"inputTranscription":{"text":"too early"}}}"#,
                )
                .is_err()
        );
        driver
            .receive(DataFrameKind::Text, br#"{"setup_complete":{}}"#)
            .expect("first completion");
        assert!(
            driver
                .receive(DataFrameKind::Text, br#"{"setupComplete":{}}"#)
                .is_err()
        );
    }

    #[test]
    fn swr_session_003_close_is_deterministic_and_idempotence_is_rejected() {
        let mut driver = SessionDriver::new(&config());
        let close: Value =
            serde_json::from_str(&driver.close().expect("first close")).expect("stream end JSON");
        assert_eq!(close["realtimeInput"]["audioStreamEnd"], true);
        assert!(driver.close().is_err());
        assert!(
            driver
                .receive(DataFrameKind::Text, br#"{"setupComplete":{}}"#)
                .is_err()
        );
    }
}
