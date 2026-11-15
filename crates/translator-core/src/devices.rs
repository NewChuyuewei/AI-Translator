//! Bounded audio-device snapshots shared by desktop adapters and UI.

use crate::{AudioSourceMode, SessionOptions};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::HashSet;

pub const MAX_DEVICES_PER_CLASS: usize = 128;
pub const MAX_DEVICE_TEXT_CHARS: usize = 1_024;
pub const MAX_PLATFORM_CHARS: usize = 32;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

impl AudioDeviceInfo {
    pub fn bounded(id: &str, name: &str, is_default: bool) -> Option<Self> {
        let id = bounded_text(id, MAX_DEVICE_TEXT_CHARS);
        if id.is_empty() {
            return None;
        }
        let name = bounded_text(name, MAX_DEVICE_TEXT_CHARS);
        Some(Self {
            name: if name.is_empty() { id.clone() } else { name },
            id,
            is_default,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    pub microphones: Vec<AudioDeviceInfo>,
    pub system_sources: Vec<AudioDeviceInfo>,
    pub playback_devices: Vec<AudioDeviceInfo>,
    pub platform: String,
}

impl AudioDevices {
    /// SWR-DEV-001/002: normalize an untrusted backend snapshot before UI use.
    pub fn bounded(
        microphones: Vec<AudioDeviceInfo>,
        system_sources: Vec<AudioDeviceInfo>,
        playback_devices: Vec<AudioDeviceInfo>,
        platform: &str,
    ) -> Self {
        Self {
            microphones: normalize(microphones),
            system_sources: normalize(system_sources),
            playback_devices: normalize(playback_devices),
            platform: bounded_text(platform, MAX_PLATFORM_CHARS),
        }
    }

    /// SWR-DEV-003: reject stale or cross-class persisted identifiers before
    /// opening any device. `None` explicitly requests the backend default.
    pub fn validate_selection(&self, options: &SessionOptions) -> Result<()> {
        if options.mode.wants_microphone() {
            validate_id(
                options.microphone_device.as_deref(),
                &self.microphones,
                "麦克风",
            )?;
        }
        if options.mode.wants_system_audio() {
            validate_id(
                options.system_device.as_deref(),
                &self.system_sources,
                "系统音频设备",
            )?;
        }
        if options.play_translation() {
            validate_id(
                options.playback_device.as_deref(),
                &self.playback_devices,
                "播放设备",
            )?;
        }
        Ok(())
    }

    pub fn total_count(&self) -> usize {
        self.microphones.len() + self.system_sources.len() + self.playback_devices.len()
    }
}

pub trait DeviceProvider {
    fn refresh(&mut self) -> Result<AudioDevices>;
}

/// Deterministic provider for UI and lifecycle verification without hardware.
pub struct MemoryDeviceProvider {
    snapshot: AudioDevices,
    fail_next: bool,
}

impl MemoryDeviceProvider {
    pub fn new(snapshot: AudioDevices) -> Self {
        Self {
            snapshot,
            fail_next: false,
        }
    }

    pub fn fail_next(&mut self) {
        self.fail_next = true;
    }
}

impl DeviceProvider for MemoryDeviceProvider {
    fn refresh(&mut self) -> Result<AudioDevices> {
        if self.fail_next {
            self.fail_next = false;
            return Err(anyhow!("无法刷新音频设备"));
        }
        Ok(self.snapshot.clone())
    }
}

fn normalize(values: Vec<AudioDeviceInfo>) -> Vec<AudioDeviceInfo> {
    let mut seen = HashSet::with_capacity(MAX_DEVICES_PER_CLASS);
    let mut default_seen = false;
    values
        .into_iter()
        .take(MAX_DEVICES_PER_CLASS.saturating_mul(4))
        .filter_map(|value| {
            let mut value = AudioDeviceInfo::bounded(&value.id, &value.name, value.is_default)?;
            if !seen.insert(value.id.clone()) {
                return None;
            }
            if value.is_default {
                if default_seen {
                    value.is_default = false;
                } else {
                    default_seen = true;
                }
            }
            Some(value)
        })
        .take(MAX_DEVICES_PER_CLASS)
        .collect()
}

fn validate_id(value: Option<&str>, devices: &[AudioDeviceInfo], label: &str) -> Result<()> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if devices.iter().any(|device| device.id == value) {
        Ok(())
    } else {
        Err(anyhow!("所选{label}已不可用，请刷新设备列表"))
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelKind, ProxyMode, ThinkingLevel, TranscriptionMode};

    fn device(id: &str, default: bool) -> AudioDeviceInfo {
        AudioDeviceInfo {
            id: id.into(),
            name: id.into(),
            is_default: default,
        }
    }

    fn options() -> SessionOptions {
        SessionOptions {
            api_key: "fixture".into(),
            model: ModelKind::GeneralLive,
            target_language: "it".into(),
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            bidirectional: false,
            other_language: "en".into(),
            mode: AudioSourceMode::Mixed,
            microphone_device: Some("mic-1".into()),
            system_device: Some("system-1".into()),
            playback_enabled: true,
            playback_device: Some("speaker-1".into()),
            token_saver: true,
            proxy_mode: ProxyMode::Direct,
            proxy_url: String::new(),
            overlay_scale: 1.0,
        }
    }

    #[test]
    fn swr_dev_001_snapshot_is_bounded_and_control_characters_are_removed() {
        let values = (0..150)
            .map(|index| device(&format!("mic-{index}\n"), false))
            .collect();
        let snapshot = AudioDevices::bounded(values, Vec::new(), Vec::new(), &"p".repeat(50));
        assert_eq!(snapshot.microphones.len(), MAX_DEVICES_PER_CLASS);
        assert!(
            snapshot
                .microphones
                .iter()
                .all(|value| !value.id.contains('\n'))
        );
        assert_eq!(snapshot.platform.chars().count(), MAX_PLATFORM_CHARS);
    }

    #[test]
    fn swr_dev_002_duplicate_ids_and_defaults_are_deterministically_normalized() {
        let snapshot = AudioDevices::bounded(
            vec![device("a", true), device("a", false), device("b", true)],
            Vec::new(),
            Vec::new(),
            "linux",
        );
        assert_eq!(snapshot.microphones.len(), 2);
        assert!(snapshot.microphones[0].is_default);
        assert!(!snapshot.microphones[1].is_default);
    }

    #[test]
    fn swr_dev_003_stale_and_cross_class_selections_are_rejected() {
        let snapshot = AudioDevices::bounded(
            vec![device("mic-1", true)],
            vec![device("system-1", true)],
            vec![device("speaker-1", true)],
            "fixture",
        );
        assert!(snapshot.validate_selection(&options()).is_ok());
        let mut stale = options();
        stale.microphone_device = Some("speaker-1".into());
        assert!(snapshot.validate_selection(&stale).is_err());
    }

    #[test]
    fn swr_dev_004_memory_provider_has_deterministic_recovery() {
        let snapshot = AudioDevices::bounded(
            vec![device("mic-1", true)],
            Vec::new(),
            Vec::new(),
            "fixture",
        );
        let mut provider = MemoryDeviceProvider::new(snapshot.clone());
        provider.fail_next();
        assert!(provider.refresh().is_err());
        assert_eq!(provider.refresh().expect("second refresh"), snapshot);
    }
}
