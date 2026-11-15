//! CPAL-backed enumeration only. This adapter never opens an audio stream.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use translator_core::devices::{AudioDeviceInfo, AudioDevices, DeviceProvider};

#[derive(Default)]
pub struct NativeDeviceProvider;

impl DeviceProvider for NativeDeviceProvider {
    fn refresh(&mut self) -> Result<AudioDevices> {
        let host = cpal::default_host();
        let input_default = host
            .default_input_device()
            .and_then(|device| device.name().ok());
        let output_default = host
            .default_output_device()
            .and_then(|device| device.name().ok());

        let microphones = host
            .input_devices()
            .context("无法枚举麦克风")?
            .filter_map(|device| device.name().ok())
            .map(|name| AudioDeviceInfo {
                id: name.clone(),
                is_default: input_default.as_ref() == Some(&name),
                name,
            })
            .collect();
        let playback_devices: Vec<AudioDeviceInfo> = host
            .output_devices()
            .context("无法枚举播放设备")?
            .filter_map(|device| device.name().ok())
            .map(|name| AudioDeviceInfo {
                id: name.clone(),
                is_default: output_default.as_ref() == Some(&name),
                name,
            })
            .collect();

        #[cfg(target_os = "windows")]
        let system_sources = playback_devices.clone();
        #[cfg(target_os = "macos")]
        let system_sources = vec![AudioDeviceInfo {
            id: "screen-capture-kit".into(),
            name: "系统音频 · ScreenCaptureKit".into(),
            is_default: true,
        }];
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let system_sources = Vec::new();

        Ok(AudioDevices::bounded(
            microphones,
            system_sources,
            playback_devices,
            std::env::consts::OS,
        ))
    }
}
