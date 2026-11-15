slint::include_modules!();

use slint::ComponentHandle;
use translator_core::{
    AudioSourceMode, ModelKind, ProxyMode, SessionOptions, ThinkingLevel, TranscriptionMode,
};

fn selected<T: Copy>(index: i32, values: &[T]) -> T {
    values.get(index.max(0) as usize).copied().unwrap_or(values[0])
}

fn options_from_ui(ui: &ControlWindow) -> SessionOptions {
    let model = selected(ui.get_model_index(), &[
        ModelKind::GeneralLive, ModelKind::ExtendedThinking,
        ModelKind::LiveTranslate35, ModelKind::TranscribeLive35,
    ]);
    let target = selected(ui.get_target_index(), &["zh", "en", "ja", "ko", "es", "fr", "de", "it"]);
    SessionOptions {
        api_key: ui.get_api_key().to_string(),
        model,
        target_language: target.into(),
        thinking_level: ThinkingLevel::Low,
        transcription_mode: TranscriptionMode::Verbatim,
        bidirectional: ui.get_bidirectional(),
        other_language: if target == "en" { "zh".into() } else { "en".into() },
        mode: selected(ui.get_source_index(), &[
            AudioSourceMode::MicrophoneOnly, AudioSourceMode::SystemAudioOnly, AudioSourceMode::Mixed,
        ]),
        microphone_device: None,
        system_device: None,
        playback_enabled: ui.get_playback_enabled() && !model.is_transcription_only(),
        playback_device: None,
        token_saver: ui.get_token_saver(),
        proxy_mode: selected(ui.get_proxy_index(), &[ProxyMode::Auto, ProxyMode::Manual, ProxyMode::Direct]),
        proxy_url: ui.get_proxy_url().to_string(),
        overlay_scale: ui.get_overlay_scale() as f64,
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let control = ControlWindow::new()?;
    let caption = CaptionWindow::new()?;
    caption.show()?;

    let weak = control.as_weak();
    control.on_start_stop(move || {
        let Some(ui) = weak.upgrade() else { return; };
        let options = options_from_ui(&ui);
        match options.validate() {
            Ok(()) => ui.set_status_message(
                "配置校验通过。Slint 原生窗口已运行；会话、设备与保险库适配器仍在迁移中，当前不会采集或上传音频。".into()
            ),
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
