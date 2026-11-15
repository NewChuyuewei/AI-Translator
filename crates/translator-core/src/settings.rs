//! Versioned ordinary settings format shared by desktop adapters.
//!
//! This module never persists API keys or proxy credentials. File-system
//! durability and permissions remain platform-adapter responsibilities.

use crate::{SessionOptions, checked_overlay_scale};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

pub const SETTINGS_VERSION: u8 = 2;
pub const MAX_SETTINGS_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSettings {
    pub version: u8,
    pub save_sensitive: bool,
    pub options: SessionOptions,
}

#[derive(Clone, Debug)]
pub struct DecodedSettings {
    pub settings: PersistedSettings,
    /// The adapter must atomically persist `settings` before considering a
    /// legacy migration complete.
    pub migration_required: bool,
}

/// SWR-SET-001: sensitive values never enter the ordinary settings document.
pub fn settings_for_storage(options: &SessionOptions, save_sensitive: bool) -> PersistedSettings {
    let mut options = options.clone();
    options.api_key.clear();
    options.proxy_url.clear();
    PersistedSettings {
        version: SETTINGS_VERSION,
        save_sensitive,
        options,
    }
}

pub fn encode_settings(settings: &PersistedSettings) -> Result<Vec<u8>> {
    if settings.version != SETTINGS_VERSION {
        return Err(anyhow!("只能编码当前版本的用户设置"));
    }
    let safe = settings_for_storage(&settings.options, settings.save_sensitive);
    serde_json::to_vec_pretty(&safe).context("无法编码用户设置")
}

/// SWR-SET-002..004: parse a bounded document, reject unknown versions, and
/// return an explicit migration transaction for legacy version 1.
pub fn decode_settings(bytes: &[u8]) -> Result<DecodedSettings> {
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(anyhow!("用户设置超过 1 MiB 安全限制"));
    }
    let mut settings: PersistedSettings =
        serde_json::from_slice(bytes).context("用户设置文件已损坏")?;
    let migration_required = match settings.version {
        1 => {
            settings.version = SETTINGS_VERSION;
            settings.save_sensitive = false;
            true
        }
        SETTINGS_VERSION => false,
        version => {
            return Err(anyhow!(
                "不支持的用户设置版本 {version}；请删除旧配置后重试"
            ));
        }
    };

    // Defense in depth for legacy and manually edited files.
    settings.options.api_key.clear();
    settings.options.proxy_url.clear();
    settings.options.overlay_scale =
        checked_overlay_scale(settings.options.overlay_scale).unwrap_or(1.0);

    Ok(DecodedSettings {
        settings,
        migration_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioSourceMode, ModelKind, ProxyMode, ThinkingLevel, TranscriptionMode};

    fn options() -> SessionOptions {
        SessionOptions {
            api_key: "PRIVATE-KEY".into(),
            model: ModelKind::GeneralLive,
            target_language: "zh".into(),
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            bidirectional: false,
            other_language: "en".into(),
            mode: AudioSourceMode::MicrophoneOnly,
            microphone_device: None,
            system_device: None,
            playback_enabled: false,
            playback_device: None,
            token_saver: true,
            proxy_mode: ProxyMode::Manual,
            proxy_url: "http://alice:password@127.0.0.1:7890".into(),
            overlay_scale: 1.0,
        }
    }

    #[test]
    fn swr_set_001_encode_never_contains_sensitive_values() {
        let settings = settings_for_storage(&options(), true);
        let bytes = encode_settings(&settings).expect("encode fixture");
        let text = String::from_utf8(bytes).expect("JSON is UTF-8");
        assert!(!text.contains("PRIVATE-KEY"));
        assert!(!text.contains("alice"));
        assert!(text.contains("\"saveSensitive\": true"));
    }

    #[test]
    fn swr_set_002_current_format_round_trips_with_compatible_fields() {
        let bytes =
            encode_settings(&settings_for_storage(&options(), false)).expect("encode fixture");
        let decoded = decode_settings(&bytes).expect("decode fixture");
        assert!(!decoded.migration_required);
        assert_eq!(decoded.settings.version, SETTINGS_VERSION);
        assert_eq!(decoded.settings.options.model, ModelKind::GeneralLive);
        assert_eq!(decoded.settings.options.proxy_mode, ProxyMode::Manual);
    }

    #[test]
    fn swr_set_003_legacy_migration_is_explicit_and_erases_secrets() {
        let mut value = serde_json::to_value(PersistedSettings {
            version: 1,
            save_sensitive: true,
            options: options(),
        })
        .expect("serialize fixture");
        value["options"]["overlayScale"] = serde_json::json!(99.0);
        let bytes = serde_json::to_vec(&value).expect("encode legacy fixture");
        let decoded = decode_settings(&bytes).expect("migrate fixture");
        assert!(decoded.migration_required);
        assert_eq!(decoded.settings.version, SETTINGS_VERSION);
        assert!(!decoded.settings.save_sensitive);
        assert!(decoded.settings.options.api_key.is_empty());
        assert!(decoded.settings.options.proxy_url.is_empty());
        assert_eq!(decoded.settings.options.overlay_scale, 1.0);
    }

    #[test]
    fn swr_set_004_rejects_oversize_corruption_and_unknown_versions() {
        assert!(decode_settings(&vec![b' '; MAX_SETTINGS_BYTES + 1]).is_err());
        assert!(decode_settings(b"not-json").is_err());
        let mut value = serde_json::to_value(settings_for_storage(&options(), false))
            .expect("serialize fixture");
        value["version"] = serde_json::json!(3);
        let bytes = serde_json::to_vec(&value).expect("encode fixture");
        assert!(decode_settings(&bytes).is_err());
    }
}
