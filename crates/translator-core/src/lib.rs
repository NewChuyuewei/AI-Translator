//! UI-independent application contracts shared by the legacy Tauri adapter and
//! the native Slint desktop. This crate deliberately has no UI, audio-backend,
//! network-runtime, or platform-framework dependency.

#![forbid(unsafe_code)]

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::Zeroize;

pub mod diagnostics;
pub mod protocol;
pub mod server_protocol;
pub mod session_driver;
pub mod session_port;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    GeneralLive,
    #[serde(alias = "live_translate")]
    ExtendedThinking,
    #[serde(rename = "live_translate_35")]
    LiveTranslate35,
    #[serde(rename = "transcribe_live_35")]
    TranscribeLive35,
}

impl ModelKind {
    pub fn is_transcription_only(self) -> bool {
        self == Self::TranscribeLive35
    }
    pub fn supports_bidirectional_instruction(self) -> bool {
        matches!(self, Self::GeneralLive | Self::ExtendedThinking)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThinkingLevel {
    #[default]
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TranscriptionMode {
    #[default]
    Verbatim,
    Smart,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Auto,
    Manual,
    Direct,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceMode {
    #[default]
    MicrophoneOnly,
    SystemAudioOnly,
    Mixed,
}

impl AudioSourceMode {
    pub fn wants_microphone(self) -> bool {
        matches!(self, Self::MicrophoneOnly | Self::Mixed)
    }
    pub fn wants_system_audio(self) -> bool {
        matches!(self, Self::SystemAudioOnly | Self::Mixed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOptions {
    pub api_key: String,
    pub model: ModelKind,
    pub target_language: String,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default)]
    pub transcription_mode: TranscriptionMode,
    #[serde(default)]
    pub bidirectional: bool,
    #[serde(default = "default_other_language")]
    pub other_language: String,
    pub mode: AudioSourceMode,
    #[serde(default)]
    pub microphone_device: Option<String>,
    #[serde(default)]
    pub system_device: Option<String>,
    #[serde(default)]
    pub playback_enabled: bool,
    #[serde(default)]
    pub playback_device: Option<String>,
    #[serde(default = "default_token_saver")]
    pub token_saver: bool,
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default = "default_overlay_scale")]
    pub overlay_scale: f64,
}

fn default_other_language() -> String {
    "en".into()
}
fn default_token_saver() -> bool {
    true
}
fn default_overlay_scale() -> f64 {
    1.0
}

impl Drop for SessionOptions {
    fn drop(&mut self) {
        self.api_key.zeroize();
        self.proxy_url.zeroize();
    }
}

impl SessionOptions {
    /// SWR-CORE-003, SWR-CORE-004: validate every untrusted UI/persistence
    /// boundary before resource acquisition without reflecting credentials.
    pub fn validate(&self) -> Result<()> {
        let key = self.api_key.trim();
        if key.is_empty() {
            return Err(anyhow!("请填写 Gemini API Key"));
        }
        if key.len() > 512 {
            return Err(anyhow!("Gemini API Key 长度无效"));
        }
        if !valid_language(&self.target_language) || !valid_language(&self.other_language) {
            return Err(anyhow!("语言代码必须是有效的 BCP-47 字母/连字符形式"));
        }
        if self.bidirectional && !self.model.supports_bidirectional_instruction() {
            return Err(anyhow!("此模型不支持双向自动切换；请选择单向翻译或转写"));
        }
        checked_overlay_scale(self.overlay_scale)?;
        if self.proxy_url.len() > 2048 || self.proxy_url.chars().any(char::is_control) {
            return Err(anyhow!("代理地址过长或含控制字符"));
        }
        if self.proxy_mode == ProxyMode::Manual {
            validate_manual_proxy(&self.proxy_url)?;
        }
        for name in [
            &self.microphone_device,
            &self.system_device,
            &self.playback_device,
        ]
        .into_iter()
        .flatten()
        {
            if name.len() > 1024 || name.chars().any(char::is_control) {
                return Err(anyhow!("音频设备名称过长或含控制字符"));
            }
        }
        if self.model.is_transcription_only() && self.playback_enabled {
            return Err(anyhow!("实时转写模型不输出译音，请关闭译音播放"));
        }
        Ok(())
    }

    pub fn play_translation(&self) -> bool {
        self.playback_enabled && !self.model.is_transcription_only()
    }
}

fn valid_language(value: &str) -> bool {
    (2..=24).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
}

pub fn checked_overlay_scale(scale: f64) -> Result<f64> {
    if !scale.is_finite() || !(0.8..=1.6).contains(&scale) {
        return Err(anyhow!("字幕缩放比例必须在 80% 到 160% 之间"));
    }
    Ok((scale * 10.0).round() / 10.0)
}

/// Structural validation only. Authentication details are intentionally never
/// included in errors. Transport-specific parsing remains in the network adapter.
pub fn validate_manual_proxy(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(anyhow!("请填写有效的代理地址"));
    }
    let url =
        Url::parse(value).map_err(|_| anyhow!("代理地址格式无效；示例 http://127.0.0.1:7890"))?;
    if !matches!(url.scheme(), "http" | "socks" | "socks5" | "socks5h") {
        return Err(anyhow!("只支持 http://、socks5:// 或 socks5h:// 代理地址"));
    }
    if url.host().is_none()
        || url.port_or_known_default().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(anyhow!("代理地址只能包含主机、端口及可选的用户名密码"));
    }
    Ok(())
}

/// SWR-CORE-001, SWR-CORE-002: all externally visible lifecycle states are
/// represented by this closed type rather than unchecked strings.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    #[default]
    Idle,
    Connecting,
    Running,
    Stopping,
    Error,
}

impl SessionPhase {
    pub fn can_start(self) -> bool {
        matches!(self, Self::Idle | Self::Error)
    }
    pub fn is_active(self) -> bool {
        matches!(self, Self::Connecting | Self::Running | Self::Stopping)
    }

    pub fn transition(self, event: SessionEvent) -> Result<Self, TransitionError> {
        let next = match (self, event) {
            (Self::Idle | Self::Error, SessionEvent::StartRequested) => Self::Connecting,
            (Self::Connecting, SessionEvent::SetupCompleted) => Self::Running,
            (Self::Connecting | Self::Running, SessionEvent::StopRequested) => Self::Stopping,
            (Self::Stopping, SessionEvent::StopCompleted) => Self::Idle,
            (Self::Connecting | Self::Running, SessionEvent::Failed) => Self::Error,
            _ => return Err(TransitionError { phase: self, event }),
        };
        Ok(next)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    StartRequested,
    SetupCompleted,
    StopRequested,
    StopCompleted,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionError {
    pub phase: SessionPhase,
    pub event: SessionEvent,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid session transition: {:?} + {:?}",
            self.phase, self.event
        )
    }
}
impl std::error::Error for TransitionError {}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub phase: SessionPhase,
    pub message: String,
    pub shortcut_available: bool,
    pub overlay_locked: bool,
    pub platform: String,
    pub network_route: String,
    pub transcription_only: bool,
    pub overlay_scale: f64,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            phase: SessionPhase::Idle,
            message: "准备就绪".into(),
            shortcut_available: false,
            overlay_locked: true,
            platform: std::env::consts::OS.into(),
            network_route: "尚未检测".into(),
            transcription_only: false,
            overlay_scale: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> SessionOptions {
        serde_json::from_value(serde_json::json!({
            "apiKey":"test", "model":"general_live", "targetLanguage":"zh",
            "mode":"microphone_only", "proxyMode":"auto"
        }))
        .unwrap()
    }

    #[test]
    fn swr_core_001_lifecycle_accepts_only_defined_nominal_transitions() {
        let phase = SessionPhase::Idle
            .transition(SessionEvent::StartRequested)
            .expect("start must connect")
            .transition(SessionEvent::SetupCompleted)
            .expect("setup must run")
            .transition(SessionEvent::StopRequested)
            .expect("stop must enter stopping")
            .transition(SessionEvent::StopCompleted)
            .expect("completion must idle");
        assert_eq!(phase, SessionPhase::Idle);
        assert!(SessionPhase::Error.can_start());
        assert!(SessionPhase::Running.is_active());
    }

    #[test]
    fn swr_core_002_invalid_lifecycle_transitions_are_rejected() {
        for (phase, event) in [
            (SessionPhase::Idle, SessionEvent::SetupCompleted),
            (SessionPhase::Idle, SessionEvent::StopRequested),
            (SessionPhase::Connecting, SessionEvent::StartRequested),
            (SessionPhase::Running, SessionEvent::StartRequested),
            (SessionPhase::Stopping, SessionEvent::SetupCompleted),
            (SessionPhase::Error, SessionEvent::StopCompleted),
        ] {
            assert_eq!(
                phase.transition(event),
                Err(TransitionError { phase, event })
            );
        }
        assert_eq!(
            SessionPhase::Connecting.transition(SessionEvent::Failed),
            Ok(SessionPhase::Error)
        );
    }

    #[test]
    fn swr_core_003_options_enforce_model_and_untrusted_input_invariants() {
        let mut value = options();
        assert!(value.validate().is_ok());
        value.model = ModelKind::TranscribeLive35;
        value.playback_enabled = true;
        assert!(value.validate().is_err());
        value.playback_enabled = false;
        value.bidirectional = true;
        assert!(value.validate().is_err());
        value.bidirectional = false;
        value.microphone_device = Some("bad\ndevice".into());
        assert!(value.validate().is_err());
    }

    #[test]
    fn swr_core_004_proxy_errors_never_echo_credentials() {
        let secret = "user:private-password";
        let error = validate_manual_proxy(&format!("ftp://{secret}@localhost:21"))
            .unwrap_err()
            .to_string();
        assert!(!error.contains(secret));
        assert!(validate_manual_proxy("http://127.0.0.1:7890").is_ok());
        assert!(validate_manual_proxy("socks5://localhost:1080").is_ok());
    }

    #[test]
    fn scale_is_bounded_and_normalized() {
        assert_eq!(checked_overlay_scale(1.26).unwrap(), 1.3);
        assert!(checked_overlay_scale(f64::NAN).is_err());
        assert!(checked_overlay_scale(1.7).is_err());
    }

    #[test]
    fn swr_core_005_serialized_contract_remains_compatible_with_existing_settings() {
        let value = options();
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["model"], "general_live");
        assert_eq!(json["mode"], "microphone_only");
        assert_eq!(json["thinkingLevel"], "LOW");
        assert_eq!(json["overlayScale"], 1.0);
        let status = serde_json::to_value(SessionStatus::default()).unwrap();
        assert_eq!(status["phase"], "idle");
    }
}
