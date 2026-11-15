//! Bounded UI/session boundary shared by native shells and test adapters.
//! PCM is intentionally absent from `UiEvent`; audio remains on the realtime
//! transport path and can never be queued for rendering.

use crate::{
    SessionEvent, SessionOptions, SessionPhase, SessionStatus,
    protocol::LiveProtocolConfig,
    server_protocol::{DataFrameKind, ParsedMessage},
    session_driver::{DriverEvent, SessionDriver},
};
use anyhow::{Result, anyhow};
use std::collections::VecDeque;

pub const MAX_UI_TEXT_CHARS: usize = 300;
pub const MAX_NOTICE_CHARS: usize = 1_200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionSnapshot {
    pub original: String,
    pub translated: String,
    pub transcription_only: bool,
}

impl CaptionSnapshot {
    pub fn bounded(original: &str, translated: &str, transcription_only: bool) -> Self {
        Self {
            original: tail_chars(original, MAX_UI_TEXT_CHARS),
            translated: tail_chars(translated, MAX_UI_TEXT_CHARS),
            transcription_only,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeText(String);

impl NoticeText {
    pub fn bounded(value: &str) -> Self {
        Self(value.chars().take(MAX_NOTICE_CHARS).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    Status(SessionStatus),
    Caption(CaptionSnapshot),
    Notice(NoticeText),
}

/// SWR-PORT-001: fixed-capacity freshness-first queue. Overflow drops exactly
/// one oldest item and increments a saturating counter; capacity never grows.
pub struct BoundedEventQueue<T, const CAPACITY: usize> {
    events: VecDeque<T>,
    dropped: u64,
}

impl<T, const CAPACITY: usize> Default for BoundedEventQueue<T, CAPACITY> {
    fn default() -> Self {
        Self {
            events: VecDeque::with_capacity(CAPACITY),
            dropped: 0,
        }
    }
}

impl<T, const CAPACITY: usize> BoundedEventQueue<T, CAPACITY> {
    pub fn push(&mut self, event: T) {
        if CAPACITY == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if self.events.len() == CAPACITY {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.events.pop_front()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Commands available to a UI shell. Implementations own network/runtime
/// details, while this contract remains free of Slint, Tauri and async runtimes.
pub trait SessionPort {
    fn start(&mut self, options: &SessionOptions) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn phase(&self) -> SessionPhase;
    fn poll_event(&mut self) -> Option<UiEvent>;
}

/// Deterministic adapter for requirements and UI integration tests. Server
/// bytes are injected by the test; no network or device is opened.
pub struct MemorySessionPort<const EVENT_CAPACITY: usize> {
    driver: Option<SessionDriver>,
    status: SessionStatus,
    events: BoundedEventQueue<UiEvent, EVENT_CAPACITY>,
}

impl<const EVENT_CAPACITY: usize> Default for MemorySessionPort<EVENT_CAPACITY> {
    fn default() -> Self {
        Self {
            driver: None,
            status: SessionStatus::default(),
            events: BoundedEventQueue::default(),
        }
    }
}

impl<const EVENT_CAPACITY: usize> MemorySessionPort<EVENT_CAPACITY> {
    pub fn setup_message(&self) -> Option<&str> {
        self.driver.as_ref().map(SessionDriver::setup_message)
    }

    pub fn inject_server_frame(&mut self, kind: DataFrameKind, bytes: &[u8]) -> Result<()> {
        let driver = self
            .driver
            .as_mut()
            .ok_or_else(|| anyhow!("会话尚未启动"))?;
        match driver.receive(kind, bytes)? {
            DriverEvent::SetupComplete => {
                let next = self.status.phase.transition(SessionEvent::SetupCompleted)?;
                self.status.phase = next;
                self.status.message = "内存会话正在运行".into();
                self.events.push(UiEvent::Status(self.status.clone()));
            }
            DriverEvent::Message(message) => self.publish_message(message),
            DriverEvent::Metadata(_) => {}
        }
        Ok(())
    }

    pub fn encode_audio(&self, pcm: &[u8]) -> Result<String> {
        self.driver
            .as_ref()
            .ok_or_else(|| anyhow!("会话尚未启动"))?
            .encode_audio(pcm)
    }

    fn publish_message(&mut self, message: ParsedMessage) {
        let original = message
            .input
            .or(message.interim_input)
            .map_or_else(String::new, |value| value.0);
        let translated = if self.status.transcription_only {
            String::new()
        } else {
            message.output.map_or_else(String::new, |value| value.0)
        };
        if !original.is_empty() || !translated.is_empty() {
            self.events.push(UiEvent::Caption(CaptionSnapshot::bounded(
                &original,
                &translated,
                self.status.transcription_only,
            )));
        }
    }
}

impl<const EVENT_CAPACITY: usize> SessionPort for MemorySessionPort<EVENT_CAPACITY> {
    fn start(&mut self, options: &SessionOptions) -> Result<()> {
        options.validate()?;
        let next = self.status.phase.transition(SessionEvent::StartRequested)?;
        self.driver = Some(SessionDriver::new(&LiveProtocolConfig::from(options)));
        self.status.phase = next;
        self.status.message = "等待 setupComplete".into();
        self.status.transcription_only = options.model.is_transcription_only();
        self.events.push(UiEvent::Status(self.status.clone()));
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let stopping = self.status.phase.transition(SessionEvent::StopRequested)?;
        self.status.phase = stopping;
        self.status.message = "正在停止".into();
        self.events.push(UiEvent::Status(self.status.clone()));
        let driver = self
            .driver
            .as_mut()
            .ok_or_else(|| anyhow!("会话驱动器缺失"))?;
        let _stream_end = driver.close()?;
        let idle = self.status.phase.transition(SessionEvent::StopCompleted)?;
        self.status.phase = idle;
        self.status.message = "已停止".into();
        self.driver = None;
        self.events.push(UiEvent::Status(self.status.clone()));
        Ok(())
    }

    fn phase(&self) -> SessionPhase {
        self.status.phase
    }

    fn poll_event(&mut self) -> Option<UiEvent> {
        self.events.pop()
    }
}

fn tail_chars(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    value.chars().skip(count.saturating_sub(limit)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModelKind;

    fn options() -> SessionOptions {
        serde_json::from_value(serde_json::json!({
            "apiKey":"test", "model":"general_live", "targetLanguage":"zh",
            "mode":"microphone_only", "proxyMode":"direct"
        }))
        .expect("valid fixture")
    }

    #[test]
    fn swr_port_001_queue_is_bounded_and_freshness_first() {
        let mut queue = BoundedEventQueue::<u8, 2>::default();
        queue.push(1);
        queue.push(2);
        queue.push(3);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.dropped(), 1);
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        let mut zero = BoundedEventQueue::<u8, 0>::default();
        zero.push(1);
        assert!(zero.is_empty());
        assert_eq!(zero.dropped(), 1);
    }

    #[test]
    fn swr_port_002_memory_port_enforces_lifecycle_and_publishes_typed_events() {
        let mut port = MemorySessionPort::<8>::default();
        port.start(&options()).expect("start fixture");
        assert_eq!(port.phase(), SessionPhase::Connecting);
        assert!(port.encode_audio(&[0, 1]).is_err());
        port.inject_server_frame(DataFrameKind::Text, br#"{"setupComplete":{}}"#)
            .expect("setup complete");
        assert_eq!(port.phase(), SessionPhase::Running);
        assert!(port.encode_audio(&[0, 1]).is_ok());
        port.inject_server_frame(
            DataFrameKind::Text,
            r#"{"serverContent":{"inputTranscription":{"text":"hello"},"outputTranscription":{"text":"你好"}}}"#.as_bytes(),
        )
        .expect("caption event");
        let events: Vec<_> = std::iter::from_fn(|| port.poll_event()).collect();
        assert!(events.iter().any(|event| matches!(event, UiEvent::Status(status) if status.phase == SessionPhase::Running)));
        assert!(events.iter().any(|event| matches!(event, UiEvent::Caption(caption) if caption.original == "hello" && caption.translated == "你好")));
        port.stop().expect("stop fixture");
        assert_eq!(port.phase(), SessionPhase::Idle);
    }

    #[test]
    fn swr_port_003_ui_text_is_bounded_and_contains_no_pcm_variant() {
        let original = format!("prefix{}", "界".repeat(MAX_UI_TEXT_CHARS + 20));
        let caption = CaptionSnapshot::bounded(&original, "translation", false);
        assert_eq!(caption.original.chars().count(), MAX_UI_TEXT_CHARS);
        assert!(caption.original.ends_with('界'));
        let variants = [
            UiEvent::Caption(caption),
            UiEvent::Notice(NoticeText::bounded("notice")),
            UiEvent::Status(SessionStatus::default()),
        ];
        assert_eq!(variants.len(), 3);
    }

    #[test]
    fn swr_port_004_transcription_mode_never_emits_translation_playback_state() {
        let mut value = options();
        value.model = ModelKind::TranscribeLive35;
        value.playback_enabled = false;
        let mut port = MemorySessionPort::<4>::default();
        port.start(&value).expect("transcription start");
        assert!(port.status.transcription_only);
        port.inject_server_frame(DataFrameKind::Text, br#"{"setupComplete":{}}"#)
            .expect("setup complete");
        port.inject_server_frame(
            DataFrameKind::Text,
            br#"{"serverContent":{"inputTranscription":{"text":"source"},"outputTranscription":{"text":"must be hidden"}}}"#,
        )
        .expect("transcription response");
        let events: Vec<_> = std::iter::from_fn(|| port.poll_event()).collect();
        assert!(events.iter().any(|event| matches!(event, UiEvent::Caption(caption) if caption.original == "source" && caption.translated.is_empty() && caption.transcription_only)));
    }
}
