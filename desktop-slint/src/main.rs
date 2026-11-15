// Slint's generated module contains its own narrowly scoped unsafe allowance.
// `deny` keeps handwritten code safe while allowing that audited dependency boundary.
#![deny(unsafe_code)]

slint::include_modules!();

mod config_store;

use anyhow::anyhow;
use config_store::ConfigStore;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};
use translator_core::{
    AudioSourceMode, ModelKind, ProxyMode, SessionOptions, SessionPhase, ThinkingLevel,
    TranscriptionMode,
    session_port::{SessionPort, UiEvent},
    settings::PersistedSettings,
    vault::SecretPayload,
};
use zeroize::Zeroize;

/// v2.1.0 migration boundary: the shell uses the final typed port now, while
/// the real socket/audio runtime remains disabled until its adapter is proven.
#[derive(Default)]
struct PendingNativeSessionPort;

impl SessionPort for PendingNativeSessionPort {
    fn start(&mut self, options: &SessionOptions) -> anyhow::Result<()> {
        options.validate()?;
        Err(anyhow!("原生会话适配器尚未完成验证；未采集或上传任何音频"))
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        Err(anyhow!("原生会话当前未运行"))
    }

    fn phase(&self) -> SessionPhase {
        SessionPhase::Idle
    }

    fn poll_event(&mut self) -> Option<UiEvent> {
        None
    }
}

fn selected<T: Copy>(index: i32, values: &[T], safe_default: T) -> T {
    values
        .get(index.max(0) as usize)
        .copied()
        .unwrap_or(safe_default)
}

fn options_from_ui(ui: &ControlWindow) -> SessionOptions {
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
        thinking_level: ThinkingLevel::Low,
        transcription_mode: TranscriptionMode::Verbatim,
        bidirectional: ui.get_bidirectional(),
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
        microphone_device: None,
        system_device: None,
        playback_enabled: ui.get_playback_enabled() && !model.is_transcription_only(),
        playback_device: None,
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
}

fn main() -> Result<(), slint::PlatformError> {
    let control = ControlWindow::new()?;
    let caption = CaptionWindow::new()?;
    caption.show()?;

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

    let session = Rc::new(RefCell::new(PendingNativeSessionPort));
    let weak = control.as_weak();
    let callback_session = Rc::clone(&session);
    control.on_start_stop(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let options = options_from_ui(&ui);
        let result = callback_session.borrow_mut().start(&options);
        match result {
            Ok(()) => ui.set_status_message("原生会话已启动".into()),
            Err(error) => ui.set_status_message(error.to_string().into()),
        }
    });

    let weak = control.as_weak();
    let callback_config = config.clone();
    control.on_save_settings(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(store) = &callback_config else {
            ui.set_status_message("无法定位配置目录；设置未保存".into());
            return;
        };
        let options = options_from_ui(&ui);
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
            let options = options_from_ui(&ui);
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
    control.on_remove_vault(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(store) = &callback_config else {
            ui.set_status_message("无法定位配置目录；保险库不可用".into());
            return;
        };
        let options = options_from_ui(&ui);
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
    control.on_refresh_devices(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_message("设备适配器尚未接入；旧 Tauri 入口仍负责生产会话。".into());
        }
    });

    let weak = control.as_weak();
    control.on_toggle_overlay_lock(move || {
        if let Some(ui) = weak.upgrade() {
            let locked = !ui.get_overlay_locked();
            ui.set_overlay_locked(locked);
            ui.set_status_message(if locked {
                "字幕窗已标记为锁定；平台级鼠标穿透适配器尚待接入。".into()
            } else {
                "字幕窗已解锁，可由窗口管理器移动。".into()
            });
        }
    });

    let caption_weak = caption.as_weak();
    control.on_overlay_scale_changed(move |scale| {
        if let Some(window) = caption_weak.upgrade() {
            window.set_caption_scale(scale.clamp(0.8, 1.6));
        }
    });

    control.on_quit_requested(|| {
        let _ = slint::quit_event_loop();
    });

    control.run()
}
