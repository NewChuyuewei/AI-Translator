// Slint's generated module contains its own narrowly scoped unsafe allowance.
// `deny` keeps handwritten code safe while allowing that audited dependency boundary.
#![deny(unsafe_code)]

slint::include_modules!();

mod config_store;
mod device_provider;
mod native_audio;
mod native_diagnostics;
mod native_playback;
mod native_session;
mod native_transport;

use anyhow::anyhow;
use caption_platform::MousePassthrough;
use config_store::ConfigStore;
use desktop_lifecycle::{InstanceError, InstanceGuard};
use device_provider::NativeDeviceProvider;
use native_diagnostics::NativeDiagnostics;
use native_session::NativeSessionPort;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use translator_core::{
    AudioSourceMode, ModelKind, ProxyMode, SessionOptions, ThinkingLevel, TranscriptionMode,
    devices::{AudioDevices, DeviceProvider},
    diagnostics::LogLevel,
    session_port::{SessionPort, UiEvent},
    settings::PersistedSettings,
    vault::SecretPayload,
};
use zeroize::Zeroize;

fn selected<T: Copy>(index: i32, values: &[T], safe_default: T) -> T {
    values
        .get(index.max(0) as usize)
        .copied()
        .unwrap_or(safe_default)
}

fn selected_device(
    index: i32,
    devices: &[translator_core::devices::AudioDeviceInfo],
) -> Option<String> {
    usize::try_from(index.saturating_sub(1))
        .ok()
        .and_then(|index| devices.get(index))
        .map(|device| device.id.clone())
}

fn options_from_ui(ui: &ControlWindow, devices: &AudioDevices) -> SessionOptions {
    let model = selected(
        ui.get_model_index(),
        &[
            ModelKind::GeneralLive,
            ModelKind::ExtendedThinking,
            ModelKind::LiveTranslate35,
            ModelKind::TranscribeLive35,
        ],
        ModelKind::GeneralLive,
    );
    let target = selected(
        ui.get_target_index(),
        &["zh", "en", "ja", "ko", "es", "fr", "de", "it"],
        "zh",
    );
    SessionOptions {
        api_key: ui.get_api_key().to_string(),
        model,
        target_language: target.into(),
        thinking_level: selected(
            ui.get_thinking_index(),
            &[
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
            ],
            ThinkingLevel::Low,
        ),
        transcription_mode: selected(
            ui.get_transcription_index(),
            &[TranscriptionMode::Verbatim, TranscriptionMode::Smart],
            TranscriptionMode::Verbatim,
        ),
        bidirectional: ui.get_bidirectional() && model.supports_bidirectional_instruction(),
        other_language: if target == "en" {
            "zh".into()
        } else {
            "en".into()
        },
        mode: selected(
            ui.get_source_index(),
            &[
                AudioSourceMode::MicrophoneOnly,
                AudioSourceMode::SystemAudioOnly,
                AudioSourceMode::Mixed,
            ],
            AudioSourceMode::MicrophoneOnly,
        ),
        microphone_device: selected_device(ui.get_microphone_index(), &devices.microphones),
        system_device: selected_device(ui.get_system_index(), &devices.system_sources),
        playback_enabled: ui.get_playback_enabled() && !model.is_transcription_only(),
        playback_device: selected_device(ui.get_playback_index(), &devices.playback_devices),
        token_saver: ui.get_token_saver(),
        proxy_mode: selected(
            ui.get_proxy_index(),
            &[ProxyMode::Auto, ProxyMode::Manual, ProxyMode::Direct],
            ProxyMode::Auto,
        ),
        proxy_url: ui.get_proxy_url().to_string(),
        overlay_scale: ui.get_overlay_scale() as f64,
    }
}

fn index_of<T: PartialEq>(value: &T, values: &[T]) -> i32 {
    values
        .iter()
        .position(|candidate| candidate == value)
        .unwrap_or(0) as i32
}

fn apply_settings(ui: &ControlWindow, settings: &PersistedSettings) {
    let options = &settings.options;
    ui.set_model_index(index_of(
        &options.model,
        &[
            ModelKind::GeneralLive,
            ModelKind::ExtendedThinking,
            ModelKind::LiveTranslate35,
            ModelKind::TranscribeLive35,
        ],
    ));
    ui.set_target_index(index_of(
        &options.target_language.as_str(),
        &["zh", "en", "ja", "ko", "es", "fr", "de", "it"],
    ));
    ui.set_thinking_index(index_of(
        &options.thinking_level,
        &[
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
        ],
    ));
    ui.set_transcription_index(index_of(
        &options.transcription_mode,
        &[TranscriptionMode::Verbatim, TranscriptionMode::Smart],
    ));
    ui.set_source_index(index_of(
        &options.mode,
        &[
            AudioSourceMode::MicrophoneOnly,
            AudioSourceMode::SystemAudioOnly,
            AudioSourceMode::Mixed,
        ],
    ));
    ui.set_proxy_index(index_of(
        &options.proxy_mode,
        &[ProxyMode::Auto, ProxyMode::Manual, ProxyMode::Direct],
    ));
    ui.set_bidirectional(options.bidirectional);
    ui.set_token_saver(options.token_saver);
    ui.set_playback_enabled(options.play_translation());
    ui.set_overlay_scale(options.overlay_scale as f32);
    ui.set_save_sensitive(settings.save_sensitive);
    ui.set_microphone_device_id(options.microphone_device.as_deref().unwrap_or("").into());
    ui.set_system_device_id(options.system_device.as_deref().unwrap_or("").into());
    ui.set_playback_device_id(options.playback_device.as_deref().unwrap_or("").into());
}

fn device_model(
    default_label: &str,
    devices: &[translator_core::devices::AudioDeviceInfo],
) -> ModelRc<SharedString> {
    let values = std::iter::once(SharedString::from(default_label))
        .chain(
            devices
                .iter()
                .map(|device| SharedString::from(&device.name)),
        )
        .collect::<Vec<_>>();
    Rc::new(VecModel::from(values)).into()
}

fn apply_diagnostics(ui: &ControlWindow, diagnostics: &NativeDiagnostics) {
    let lines = diagnostics.lines();
    let count = lines.len() as i32;
    let model = if lines.is_empty() {
        vec![SharedString::from("尚无诊断记录")]
    } else {
        lines.into_iter().map(SharedString::from).collect()
    };
    ui.set_diagnostic_count(count);
    ui.set_diagnostic_model(Rc::new(VecModel::from(model)).into());
    ui.set_diagnostic_index(count.saturating_sub(1));
}

fn restored_device_index(id: &str, devices: &[translator_core::devices::AudioDeviceInfo]) -> i32 {
    devices
        .iter()
        .position(|device| device.id == id)
        .and_then(|index| i32::try_from(index + 1).ok())
        .unwrap_or(0)
}

fn capture_device_ids(ui: &ControlWindow, devices: &AudioDevices) {
    ui.set_microphone_device_id(
        selected_device(ui.get_microphone_index(), &devices.microphones)
            .unwrap_or_default()
            .into(),
    );
    ui.set_system_device_id(
        selected_device(ui.get_system_index(), &devices.system_sources)
            .unwrap_or_default()
            .into(),
    );
    ui.set_playback_device_id(
        selected_device(ui.get_playback_index(), &devices.playback_devices)
            .unwrap_or_default()
            .into(),
    );
}

fn apply_devices(ui: &ControlWindow, devices: &AudioDevices) {
    ui.set_microphone_model(device_model("系统默认麦克风", &devices.microphones));
    ui.set_system_model(device_model("系统默认音频源", &devices.system_sources));
    ui.set_playback_model(device_model("系统默认播放设备", &devices.playback_devices));
    ui.set_microphone_index(restored_device_index(
        ui.get_microphone_device_id().as_str(),
        &devices.microphones,
    ));
    ui.set_system_index(restored_device_index(
        ui.get_system_device_id().as_str(),
        &devices.system_sources,
    ));
    ui.set_playback_index(restored_device_index(
        ui.get_playback_device_id().as_str(),
        &devices.playback_devices,
    ));
}

/// Apply lock transitions transactionally. Enabling pass-through may fail
/// closed (the caption remains interactive but immovable). Disabling must
/// succeed before the only recovery control reports an unlocked state.
fn apply_caption_lock(
    caption: &CaptionWindow,
    input: &mut MousePassthrough,
    locked: bool,
) -> Result<(), SharedString> {
    if locked {
        caption.set_locked(true);
        input
            .set(&caption.window().window_handle(), true)
            .map_err(|error| SharedString::from(error.to_string()))
    } else {
        input
            .set(&caption.window().window_handle(), false)
            .map_err(|error| SharedString::from(error.to_string()))?;
        caption.set_locked(false);
        Ok(())
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let lock_path = match ConfigStore::prepare_instance_lock() {
        Ok(path) => path,
        Err(_) => {
            eprintln!("无法建立单实例保护；已拒绝启动");
            return Ok(());
        }
    };
    let _instance_guard = match InstanceGuard::acquire(&lock_path) {
        Ok(guard) => guard,
        Err(InstanceError::AlreadyRunning) => return Ok(()),
        Err(InstanceError::NativeOperationFailed) => {
            eprintln!("无法建立单实例保护；已拒绝启动");
            return Ok(());
        }
    };

    let control = ControlWindow::new()?;
    let caption = CaptionWindow::new()?;
    let tray = AppTray::new()?;
    caption.show()?;
    tray.show()?;

    let config = match ConfigStore::discover() {
        Ok(store) => {
            let store = Rc::new(store);
            match store.load_settings() {
                Ok(Some(settings)) => apply_settings(&control, &settings),
                Ok(None) => {}
                Err(error) => control.set_status_message(error.to_string().into()),
            }
            match store.vault_exists() {
                Ok(exists) => control.set_vault_exists(exists),
                Err(error) => control.set_status_message(error.to_string().into()),
            }
            Some(store)
        }
        Err(error) => {
            control.set_status_message(error.to_string().into());
            None
        }
    };

    let session = Rc::new(RefCell::new(NativeSessionPort::default()));
    let caption_input = Rc::new(RefCell::new(MousePassthrough::default()));
    let diagnostics = Rc::new(RefCell::new(NativeDiagnostics::default()));
    diagnostics
        .borrow()
        .record(LogLevel::Info, "Slint 原生桌面已启动");
    apply_diagnostics(&control, &diagnostics.borrow());
    let device_provider = Rc::new(RefCell::new(NativeDeviceProvider));
    let devices = Rc::new(RefCell::new(AudioDevices::default()));
    let probe_cancel = Arc::new(AtomicBool::new(false));
    match device_provider.borrow_mut().refresh() {
        Ok(snapshot) => {
            apply_devices(&control, &snapshot);
            *devices.borrow_mut() = snapshot;
        }
        Err(error) => control.set_status_message(error.to_string().into()),
    }
    let weak = control.as_weak();
    let callback_session = Rc::clone(&session);
    let callback_devices = Rc::clone(&devices);
    let callback_diagnostics = Rc::clone(&diagnostics);
    control.on_start_stop(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let result = if callback_session.borrow().phase().is_active() {
            callback_session.borrow_mut().stop()
        } else {
            let options = options_from_ui(&ui, &callback_devices.borrow());
            callback_diagnostics.borrow_mut().begin_boundary(
                &options.api_key,
                &options.proxy_url,
                "请求启动原生会话",
            );
            callback_devices
                .borrow()
                .validate_selection(&options)
                .and_then(|()| callback_session.borrow_mut().start(&options))
        };
        if let Err(error) = result {
            callback_diagnostics
                .borrow()
                .record(LogLevel::Error, &error.to_string());
            ui.set_status_message(error.to_string().into());
        }
        apply_diagnostics(&ui, &callback_diagnostics.borrow());
    });

    let weak = control.as_weak();
    let callback_devices = Rc::clone(&devices);
    let callback_cancel = Arc::clone(&probe_cancel);
    let callback_diagnostics = Rc::clone(&diagnostics);
    control.on_probe_transport(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if ui.get_probe_running() {
            callback_cancel.store(true, Ordering::Release);
            ui.set_status_message("正在取消网络验证…".into());
            return;
        }
        callback_cancel.store(false, Ordering::Release);
        let options = options_from_ui(&ui, &callback_devices.borrow());
        callback_diagnostics.borrow_mut().begin_boundary(
            &options.api_key,
            &options.proxy_url,
            "请求执行无音频网络验证",
        );
        apply_diagnostics(&ui, &callback_diagnostics.borrow());
        if let Err(error) = callback_devices.borrow().validate_selection(&options) {
            callback_diagnostics
                .borrow()
                .record(LogLevel::Error, &error.to_string());
            apply_diagnostics(&ui, &callback_diagnostics.borrow());
            ui.set_status_message(error.to_string().into());
            return;
        }
        ui.set_probe_running(true);
        ui.set_status_message("正在验证代理、TLS、WebSocket 和 setup；不会打开或上传音频".into());
        let thread_weak = weak.clone();
        let thread_cancel = Arc::clone(&callback_cancel);
        std::thread::spawn(move || {
            let result = native_transport::probe_blocking(options, thread_cancel);
            let _ = thread_weak.upgrade_in_event_loop(move |ui| {
                ui.set_probe_running(false);
                match result {
                    Ok(report) => ui.set_status_message(
                        format!(
                            "网络验证成功：{}，{} ms；未打开或上传音频",
                            report.route_label, report.elapsed_ms
                        )
                        .into(),
                    ),
                    Err(error) => ui.set_status_message(error.to_string().into()),
                }
            });
        });
    });

    let weak = control.as_weak();
    let callback_config = config.clone();
    let callback_devices = Rc::clone(&devices);
    control.on_save_settings(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(store) = &callback_config else {
            ui.set_status_message("无法定位配置目录；设置未保存".into());
            return;
        };
        let options = options_from_ui(&ui, &callback_devices.borrow());
        let save_sensitive = ui.get_save_sensitive();
        let mut password = ui.get_vault_password().to_string();
        ui.set_vault_password("".into());
        let result = if save_sensitive {
            if password.is_empty() {
                Err(anyhow!("加密保存需要输入至少 8 个字符的密码"))
            } else {
                let payload = SecretPayload {
                    api_key: options.api_key.clone(),
                    proxy_url: options.proxy_url.clone(),
                };
                store
                    .save_vault(&password, &payload)
                    .and_then(|()| store.save_settings(&options, true))
            }
        } else {
            store
                .save_settings(&options, false)
                .and_then(|()| store.remove_vault())
        };
        password.zeroize();
        match result {
            Ok(()) => {
                ui.set_vault_exists(save_sensitive);
                ui.set_status_message("设置已安全保存".into());
            }
            Err(error) => ui.set_status_message(error.to_string().into()),
        }
    });

    let weak = control.as_weak();
    let callback_config = config.clone();
    let callback_devices = Rc::clone(&devices);
    control.on_unlock_vault(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(store) = &callback_config else {
            ui.set_status_message("无法定位配置目录；保险库不可用".into());
            return;
        };
        let mut password = ui.get_vault_password().to_string();
        ui.set_vault_password("".into());
        let result = if ui.get_vault_exists() {
            store.unlock_vault(&password).map(|payload| {
                ui.set_api_key(payload.api_key.as_str().into());
                ui.set_proxy_url(payload.proxy_url.as_str().into());
                ui.set_save_sensitive(true);
            })
        } else {
            let options = options_from_ui(&ui, &callback_devices.borrow());
            let payload = SecretPayload {
                api_key: options.api_key.clone(),
                proxy_url: options.proxy_url.clone(),
            };
            store
                .save_vault(&password, &payload)
                .and_then(|()| store.save_settings(&options, true))
                .map(|()| {
                    ui.set_vault_exists(true);
                    ui.set_save_sensitive(true);
                })
        };
        password.zeroize();
        match result {
            Ok(()) => ui.set_status_message("加密保险库已解锁".into()),
            Err(error) => ui.set_status_message(error.to_string().into()),
        }
    });

    let weak = control.as_weak();
    let callback_config = config.clone();
    let callback_devices = Rc::clone(&devices);
    control.on_remove_vault(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(store) = &callback_config else {
            ui.set_status_message("无法定位配置目录；保险库不可用".into());
            return;
        };
        let options = options_from_ui(&ui, &callback_devices.borrow());
        match store
            .save_settings(&options, false)
            .and_then(|()| store.remove_vault())
        {
            Ok(()) => {
                ui.set_api_key("".into());
                ui.set_proxy_url("".into());
                ui.set_save_sensitive(false);
                ui.set_vault_exists(false);
                ui.set_vault_password("".into());
                ui.set_status_message("加密保险库已删除；普通设置不含敏感信息".into());
            }
            Err(error) => ui.set_status_message(error.to_string().into()),
        }
    });

    let weak = control.as_weak();
    let callback_provider = Rc::clone(&device_provider);
    let callback_devices = Rc::clone(&devices);
    control.on_refresh_devices(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        capture_device_ids(&ui, &callback_devices.borrow());
        match callback_provider.borrow_mut().refresh() {
            Ok(snapshot) => {
                apply_devices(&ui, &snapshot);
                ui.set_status_message(
                    format!(
                        "已刷新 {} 个有界设备（平台 {}）；尚未打开音频流",
                        snapshot.total_count(),
                        snapshot.platform
                    )
                    .into(),
                );
                *callback_devices.borrow_mut() = snapshot;
            }
            Err(error) => ui.set_status_message(error.to_string().into()),
        }
    });

    let initial_control = control.as_weak();
    let initial_caption = caption.as_weak();
    let initial_input = Rc::clone(&caption_input);
    Timer::single_shot(Duration::from_millis(1), move || {
        let (Some(ui), Some(window)) = (initial_control.upgrade(), initial_caption.upgrade())
        else {
            return;
        };
        let locked = ui.get_overlay_locked();
        if let Err(error) = apply_caption_lock(&window, &mut initial_input.borrow_mut(), locked) {
            ui.set_status_message(format!("字幕窗保持可交互的安全状态：{error}").into());
        }
    });

    let weak = control.as_weak();
    let caption_weak = caption.as_weak();
    let callback_input = Rc::clone(&caption_input);
    control.on_toggle_overlay_lock(move || {
        let (Some(ui), Some(window)) = (weak.upgrade(), caption_weak.upgrade()) else {
            return;
        };
        let requested_lock = !ui.get_overlay_locked();
        match apply_caption_lock(&window, &mut callback_input.borrow_mut(), requested_lock) {
            Ok(()) => {
                ui.set_overlay_locked(requested_lock);
                ui.set_status_message(if requested_lock {
                    "字幕窗已锁定并启用鼠标穿透。使用控制窗解锁。".into()
                } else {
                    "字幕窗已解锁；鼠标穿透已关闭，可拖动移动。".into()
                });
            }
            Err(error) if requested_lock => {
                // Fail closed: lock movement but never claim pass-through.
                ui.set_overlay_locked(true);
                ui.set_status_message(format!("字幕窗已锁定但保持可交互：{error}").into());
            }
            Err(error) => {
                // A failed disable must not create a false unlocked state.
                ui.set_overlay_locked(true);
                ui.set_status_message(
                    format!("鼠标穿透关闭失败，字幕窗仍保持锁定：{error}").into(),
                );
            }
        }
    });

    let caption_weak = caption.as_weak();
    control.on_overlay_scale_changed(move |scale| {
        if let Some(window) = caption_weak.upgrade() {
            window.set_caption_scale(scale.clamp(0.8, 1.6));
        }
    });

    let weak = control.as_weak();
    let callback_diagnostics = Rc::clone(&diagnostics);
    control.on_copy_diagnostics(move || {
        if let Some(ui) = weak.upgrade() {
            match callback_diagnostics.borrow().copy_to_clipboard() {
                Ok(()) => ui.set_status_message("已复制脱敏诊断文本".into()),
                Err(error) => ui.set_status_message(error.to_string().into()),
            }
        }
    });

    let weak = control.as_weak();
    let callback_diagnostics = Rc::clone(&diagnostics);
    control.on_clear_diagnostics(move || {
        if let Some(ui) = weak.upgrade() {
            let watermark = callback_diagnostics.borrow().clear();
            apply_diagnostics(&ui, &callback_diagnostics.borrow());
            ui.set_status_message(format!("诊断已清空（水位线 {watermark}）").into());
        }
    });

    // SWR-NLIFE-001: native close hides to tray; only explicit Quit tears
    // down the event loop and active resources.
    control
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    let control_weak = control.as_weak();
    tray.on_show_control(move || {
        if let Some(ui) = control_weak.upgrade() {
            let _ = ui.show();
            ui.window().request_redraw();
        }
    });

    let control_weak = control.as_weak();
    tray.on_toggle_session(move || {
        if let Some(ui) = control_weak.upgrade() {
            ui.invoke_start_stop();
        }
    });

    let event_control = control.as_weak();
    let event_caption = caption.as_weak();
    let event_tray = tray.as_weak();
    let event_session = Rc::clone(&session);
    let event_diagnostics = Rc::clone(&diagnostics);
    let session_timer = Timer::default();
    session_timer.start(TimerMode::Repeated, Duration::from_millis(25), move || {
        for _ in 0..32 {
            let Some(event) = event_session.borrow_mut().poll_event() else {
                break;
            };
            match event {
                UiEvent::Status(status) => {
                    let level = if status.phase == translator_core::SessionPhase::Error {
                        LogLevel::Error
                    } else {
                        LogLevel::Info
                    };
                    event_diagnostics.borrow().record(level, &status.message);
                    if let Some(tray) = event_tray.upgrade() {
                        tray.set_session_running(status.phase.is_active());
                    }
                    if let Some(ui) = event_control.upgrade() {
                        ui.set_session_running(status.phase.is_active());
                        ui.set_status_message(status.message.into());
                        apply_diagnostics(&ui, &event_diagnostics.borrow());
                    }
                }
                UiEvent::Caption(caption) => {
                    if let Some(window) = event_caption.upgrade() {
                        window.set_original_text(caption.original.into());
                        window.set_translated_text(caption.translated.into());
                        window.set_transcription_only(caption.transcription_only);
                    }
                }
                UiEvent::Notice(message) => {
                    event_diagnostics
                        .borrow()
                        .record(LogLevel::Warn, message.as_str());
                    if let Some(ui) = event_control.upgrade() {
                        ui.set_status_message(message.as_str().into());
                        apply_diagnostics(&ui, &event_diagnostics.borrow());
                    }
                }
            }
        }
    });

    let quit_cancel = Arc::clone(&probe_cancel);
    let quit_session = Rc::clone(&session);
    let request_quit = Rc::new(move || {
        quit_cancel.store(true, Ordering::Release);
        let session_active = quit_session.borrow().phase().is_active();
        if session_active {
            let _ = quit_session.borrow_mut().stop();
        }
        let _ = slint::quit_event_loop();
    });

    let control_quit = Rc::clone(&request_quit);
    control.on_quit_requested(move || control_quit());
    let tray_quit = Rc::clone(&request_quit);
    tray.on_quit_requested(move || tray_quit());

    control.show()?;
    slint::run_event_loop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use translator_core::devices::AudioDeviceInfo;

    fn device(id: &str) -> AudioDeviceInfo {
        AudioDeviceInfo {
            id: id.into(),
            name: id.into(),
            is_default: false,
        }
    }

    #[test]
    fn swr_nui_001_protocol_option_indices_round_trip() {
        let thinking = [
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
        ];
        let transcription = [TranscriptionMode::Verbatim, TranscriptionMode::Smart];
        for (index, value) in thinking.into_iter().enumerate() {
            assert_eq!(index_of(&value, &thinking), index as i32);
            assert_eq!(selected(index as i32, &thinking, ThinkingLevel::Low), value);
        }
        for (index, value) in transcription.into_iter().enumerate() {
            assert_eq!(index_of(&value, &transcription), index as i32);
            assert_eq!(
                selected(index as i32, &transcription, TranscriptionMode::Verbatim),
                value
            );
        }
    }

    #[test]
    fn swr_ndev_002_ui_indices_preserve_backend_identifiers() {
        let devices = vec![device("mic-a"), device("mic-b")];
        assert_eq!(restored_device_index("", &devices), 0);
        assert_eq!(restored_device_index("mic-b", &devices), 2);
        assert_eq!(selected_device(0, &devices), None);
        assert_eq!(selected_device(2, &devices).as_deref(), Some("mic-b"));
        assert_eq!(selected_device(99, &devices), None);
    }
}
