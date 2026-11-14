mod audio_player;
mod audio_router;
mod diagnostics;
mod gemini_client;
mod proxy;
#[cfg(target_os = "macos")] mod mac_system_audio;

use anyhow::{anyhow, Context, Result};
use audio_player::AudioPlayer;
use base64::{engine::general_purpose::STANDARD, Engine};
use audio_router::{AudioConfig, AudioRouter, AudioSourceMode};
use diagnostics::{LogEntry, LogStore, Trace};
use futures_util::{SinkExt, StreamExt};
use gemini_client::{GeminiOptions, InteractionStatus, ModelKind, ServerFrame, ThinkingLevel, TranscriptionMode};
use proxy::ProxyMode;
use ring::{aead, pbkdf2, rand::{SecureRandom, SystemRandom}};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, num::NonZeroU32, path::{Path, PathBuf}, sync::{Mutex, atomic::{AtomicU64, Ordering}}, time::Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartOptions {
    api_key: String, model: ModelKind, target_language: String,
    #[serde(default)] thinking_level: ThinkingLevel,
    #[serde(default)] transcription_mode: TranscriptionMode,
    #[serde(default)] bidirectional: bool,
    #[serde(default = "default_other")] other_language: String,
    mode: AudioSourceMode,
    #[serde(default)] microphone_device: Option<String>,
    #[serde(default)] system_device: Option<String>,
    #[serde(default)] playback_enabled: bool,
    #[serde(default)] playback_device: Option<String>,
    #[serde(default)] proxy_mode: ProxyMode,
    #[serde(default)] proxy_url: String,
    #[serde(default = "default_overlay_scale")] overlay_scale: f64,
}
fn default_other() -> String { "en".into() }
fn default_overlay_scale() -> f64 { 1.0 }
impl StartOptions {
    fn gemini(&self) -> GeminiOptions { GeminiOptions { api_key: self.api_key.clone(), model: self.model, thinking_level: self.thinking_level, transcription_mode: self.transcription_mode, target_language: self.target_language.clone(), bidirectional: self.bidirectional, other_language: self.other_language.clone() } }
    fn audio(&self) -> AudioConfig { AudioConfig { mode: self.mode, microphone_device: self.microphone_device.clone(), system_device: self.system_device.clone() } }
    fn validate(&self) -> Result<()> {
        self.gemini().validate()?;
        if self.model.is_transcription_only() && self.playback_enabled {
            return Err(anyhow!("实时转写模型不输出译音，请关闭译音播放"));
        }
        proxy::validate(self.proxy_mode, &self.proxy_url)
    }
    fn play_translation(&self) -> bool { self.playback_enabled && !self.model.is_transcription_only() }
}

const SETTINGS_VERSION: u8 = 2;
const SETTINGS_FILE: &str = "settings.json";
const SECRETS_FILE: &str = "secrets.enc.json";
const SECRET_AAD: &[u8] = b"com.gemini.live.translator/secrets/v1";
const PBKDF2_ITERATIONS: u32 = 210_000;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    version: u8,
    save_sensitive: bool,
    options: StartOptions,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretPayload { api_key: String, proxy_url: String }

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedSecrets {
    version: u8,
    algorithm: String,
    iterations: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

fn settings_for_storage(options: &StartOptions, save_sensitive: bool) -> PersistedSettings {
    let mut options = options.clone();
    // Secrets never belong to the regular preferences file. The opt-in only
    // controls whether a separate password-encrypted vault is maintained.
    options.api_key.clear();
    options.proxy_url.clear();
    PersistedSettings { version: SETTINGS_VERSION, save_sensitive, options }
}

fn config_path(app: &AppHandle, name: &str) -> Result<PathBuf> {
    Ok(app.path().app_config_dir().context("无法定位应用配置目录")?.join(name))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("应用配置目录无效"))?;
    fs::create_dir_all(parent).context("无法创建应用配置目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).context("无法限制应用配置目录权限")?;
    }
    let temporary = path.with_extension("tmp");
    let mut open = fs::OpenOptions::new();
    open.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let mut file = open.open(&temporary).context("无法写入应用配置")?;
    file.write_all(bytes).context("无法写入应用配置")?;
    file.sync_all().context("无法同步应用配置")?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).context("无法限制配置文件权限")?;
    }
    if let Err(first) = fs::rename(&temporary, path) {
        if path.exists() {
            fs::remove_file(path).context("无法替换旧的应用配置")?;
            fs::rename(&temporary, path).context("无法保存新的应用配置")?;
        } else {
            return Err(first).context("无法保存应用配置");
        }
    }
    Ok(())
}

fn write_settings(app: &AppHandle, settings: &PersistedSettings) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(settings).context("无法编码用户设置")?;
    write_private(&config_path(app, SETTINGS_FILE)?, &bytes)
}

fn read_settings(app: &AppHandle) -> Result<Option<PersistedSettings>> {
    let path = config_path(app, SETTINGS_FILE)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("无法读取用户设置"),
    };
    let mut settings: PersistedSettings = serde_json::from_slice(&bytes).context("用户设置文件已损坏")?;
    if settings.version == 1 {
        // 0.1.5 development snapshots briefly stored opted-in secrets in this
        // file. Remove them during migration; they cannot be encrypted without
        // asking for a password, so the user must enter them once more.
        settings.version = SETTINGS_VERSION;
        settings.save_sensitive = false;
        settings.options.api_key.clear();
        settings.options.proxy_url.clear();
        write_settings(app, &settings).context("无法迁移旧用户设置")?;
    } else if settings.version != SETTINGS_VERSION {
        return Err(anyhow!("不支持的用户设置版本 {}；请删除旧配置后重试", settings.version));
    }
    // Defense in depth for hand-edited files.
    settings.options.api_key.clear();
    settings.options.proxy_url.clear();
    settings.options.overlay_scale = checked_overlay_scale(settings.options.overlay_scale).unwrap_or(1.0);
    Ok(Some(settings))
}

fn validate_vault_password(password: &str) -> Result<()> {
    if password.as_bytes().len() < 8 { return Err(anyhow!("加密密码至少需要 8 个字符")); }
    if password.as_bytes().len() > 1024 { return Err(anyhow!("加密密码过长")); }
    Ok(())
}

fn derive_vault_key(password: &str, salt: &[u8], iterations: u32) -> Result<[u8; 32]> {
    let rounds = NonZeroU32::new(iterations).ok_or_else(|| anyhow!("加密配置迭代次数无效"))?;
    let mut key = [0u8; 32];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, rounds, salt, password.as_bytes(), &mut key);
    Ok(key)
}

fn encrypt_secrets(password: &str, payload: &SecretPayload) -> Result<Vec<u8>> {
    validate_vault_password(password)?;
    if payload.api_key.trim().is_empty() { return Err(anyhow!("保存加密配置前请填写 API Key")); }
    let random = SystemRandom::new();
    let mut salt = [0u8; 16]; let mut nonce = [0u8; 12];
    random.fill(&mut salt).map_err(|_| anyhow!("无法生成加密随机数"))?;
    random.fill(&mut nonce).map_err(|_| anyhow!("无法生成加密随机数"))?;
    let key_bytes = derive_vault_key(password, &salt, PBKDF2_ITERATIONS)?;
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes).map_err(|_| anyhow!("无法初始化加密器"))?);
    let mut plaintext = serde_json::to_vec(payload).context("无法编码敏感配置")?;
    key.seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(SECRET_AAD), &mut plaintext)
        .map_err(|_| anyhow!("无法加密敏感配置"))?;
    let encrypted = EncryptedSecrets {
        version: 1, algorithm: "AES-256-GCM/PBKDF2-HMAC-SHA256".into(), iterations: PBKDF2_ITERATIONS,
        salt: STANDARD.encode(salt), nonce: STANDARD.encode(nonce), ciphertext: STANDARD.encode(plaintext),
    };
    serde_json::to_vec_pretty(&encrypted).context("无法编码加密配置")
}

fn decrypt_secrets(password: &str, bytes: &[u8]) -> Result<SecretPayload> {
    validate_vault_password(password)?;
    let encrypted: EncryptedSecrets = serde_json::from_slice(bytes).map_err(|_| anyhow!("加密配置已损坏"))?;
    if encrypted.version != 1 || encrypted.algorithm != "AES-256-GCM/PBKDF2-HMAC-SHA256"
        || !(100_000..=2_000_000).contains(&encrypted.iterations) {
        return Err(anyhow!("不支持的加密配置格式"));
    }
    let salt = STANDARD.decode(&encrypted.salt).map_err(|_| anyhow!("加密配置已损坏"))?;
    let nonce_bytes = STANDARD.decode(&encrypted.nonce).map_err(|_| anyhow!("加密配置已损坏"))?;
    let nonce: [u8; 12] = nonce_bytes.try_into().map_err(|_| anyhow!("加密配置已损坏"))?;
    let mut ciphertext = STANDARD.decode(&encrypted.ciphertext).map_err(|_| anyhow!("加密配置已损坏"))?;
    let key_bytes = derive_vault_key(password, &salt, encrypted.iterations)?;
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes).map_err(|_| anyhow!("无法初始化解密器"))?);
    let plaintext = key.open_in_place(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(SECRET_AAD), &mut ciphertext)
        .map_err(|_| anyhow!("密码错误或加密配置已损坏"))?;
    serde_json::from_slice(plaintext).map_err(|_| anyhow!("密码错误或加密配置已损坏"))
}

fn write_encrypted_secrets(app: &AppHandle, password: &str, payload: &SecretPayload) -> Result<()> {
    let bytes = encrypt_secrets(password, payload)?;
    write_private(&config_path(app, SECRETS_FILE)?, &bytes)
}

fn remove_encrypted_secrets(app: &AppHandle) -> Result<()> {
    let path = config_path(app, SECRETS_FILE)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("无法删除加密配置"),
    }
}

fn read_encrypted_secrets(app: &AppHandle, password: &str) -> Result<SecretPayload> {
    let bytes = fs::read(config_path(app, SECRETS_FILE)?).context("没有找到加密配置；请填写 API Key 后重新保存")?;
    decrypt_secrets(password, &bytes)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status { phase: String, message: String, shortcut_available: bool, overlay_locked: bool, platform: String, network_route: String, transcription_only: bool, overlay_scale: f64 }
impl Default for Status {
    fn default() -> Self { Self { phase:"idle".into(), message:"准备就绪".into(), shortcut_available:false, overlay_locked:true, platform: std::env::consts::OS.into(), network_route:"尚未检测".into(), transcription_only:false, overlay_scale:1.0 } }
}
struct Inner { status: Status, last: Option<StartOptions>, cancel: Option<watch::Sender<bool>>, generation: u64 }
struct Controller(Mutex<Inner>);
// The native Android service can outlive an Activity. IDs must remain
// monotonic across Activity/controller recreation within the same process.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
impl Default for Controller {
    fn default() -> Self { Self(Mutex::new(Inner { status:Status::default(),last:None,cancel:None,generation:0 })) }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Subtitle { original: String, translated: String, transcription_only: bool }
fn checked_overlay_scale(scale: f64) -> Result<f64> {
    if !scale.is_finite() || !(0.8..=1.6).contains(&scale) {
        return Err(anyhow!("字幕缩放比例必须在 80% 到 160% 之间"));
    }
    Ok((scale * 10.0).round() / 10.0)
}
fn resize_overlay(app: &AppHandle, scale: f64) -> Result<()> {
    #[cfg(not(any(target_os="android", target_os="ios")))]
    if let Some(window) = app.get_webview_window("main") {
        window.set_size(tauri::LogicalSize::new(760.0 * scale, 138.0 * scale)).context("无法调整字幕窗口大小")?;
    }
    #[cfg(any(target_os="android", target_os="ios"))]
    let _ = (app, scale);
    Ok(())
}
fn publish(app: &AppHandle, status: &Status) { let _ = app.emit("session-status", status.clone()); }
fn set_phase(app: &AppHandle, generation: u64, phase: &str, message: &str) {
    let state = app.state::<Controller>(); let mut inner = state.0.lock().unwrap();
    if inner.generation != generation { return; }
    // Stop requested while a setup callback was in flight: don't flip back to running.
    if inner.status.phase == "stopping" && phase == "running" { return; }
    inner.status.phase = phase.into(); inner.status.message = message.into();
    let status = inner.status.clone(); drop(inner); publish(app, &status);
}
fn set_network_route(app: &AppHandle, generation: u64, route: &str) {
    let state = app.state::<Controller>(); let mut inner = state.0.lock().unwrap();
    if inner.generation != generation || inner.status.phase != "connecting" { return; }
    inner.status.network_route = route.into();
    let status = inner.status.clone(); drop(inner); publish(app, &status);
}
fn start_session(app: AppHandle, options: StartOptions) -> Result<()> {
    if let Err(error) = options.validate() {
        let trace = Trace::for_app(app.clone(), 0, &options.api_key, &options.proxy_url);
        trace.warn("开始失败：连接配置校验未通过，请查看界面提示");
        return Err(error);
    }
    let controller = app.state::<Controller>();
    let (cancel_rx, generation, status) = {
        let mut inner = controller.0.lock().unwrap();
        if matches!(inner.status.phase.as_str(), "connecting"|"running"|"stopping") { return Err(anyhow!("语音会话已经启动或正在停止")); }
        let (tx, rx) = watch::channel(false);
        inner.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed); inner.cancel = Some(tx); inner.last = Some(options.clone());
        inner.status.phase = "connecting".into(); inner.status.message = "正在连接 Gemini 并准备音频…".into();
        inner.status.network_route = "正在选择网络路径…".into();
        inner.status.transcription_only = options.model.is_transcription_only();
        (rx, inner.generation, inner.status.clone())
    };
    publish(&app, &status);
    // Clear the prior session before its first new transcript arrives.
    let _ = app.emit("subtitle-update", Subtitle { original: String::new(), translated: String::new(), transcription_only: status.transcription_only });
    let trace = Trace::for_app(app.clone(), generation, &options.api_key, &options.proxy_url);
    let proxy_mode = match options.proxy_mode { ProxyMode::Auto => "自动", ProxyMode::Manual => "手动", ProxyMode::Direct => "系统网络" };
    trace.info(format!("开始会话：版本 {}，平台 {}，模型 {:?}，音源 {:?}，网络模式 {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, options.model, options.mode, proxy_mode));
    tauri::async_runtime::spawn(async move {
        let mut cancel_rx = cancel_rx;
        let result = session(&app, generation, options, &mut cancel_rx, &trace).await;
        let requested_stop = *cancel_rx.borrow();
        if requested_stop || result.is_ok() { trace.info("会话已停止"); }
        else { trace.error("会话结束：发生错误；请查看前面的阶段记录及界面提示"); }
        let controller = app.state::<Controller>();
        let mut inner = controller.0.lock().unwrap();
        if inner.generation == generation {
            inner.cancel = None;
            if requested_stop || result.is_ok() {
                inner.status.phase = "idle".into(); inner.status.message = "已停止".into();
            } else {
                inner.status.phase = "error".into();
                inner.status.message = trace.redact(&format!("{:#}", result.err().unwrap()));
            }
            if inner.status.network_route == "正在选择网络路径…" { inner.status.network_route = "未选定".into(); }
            let status = inner.status.clone(); drop(inner); publish(&app, &status);
        }
    });
    Ok(())
}
fn stop_session(app: &AppHandle) {
    let state = app.state::<Controller>(); let mut inner = state.0.lock().unwrap();
    if let Some(cancel) = &inner.cancel {
        let _ = cancel.send(true); inner.status.phase = "stopping".into(); inner.status.message = "正在停止并释放音频…".into();
        #[cfg(target_os="android")] let generation = inner.generation;
        let status = inner.status.clone(); drop(inner); publish(app, &status);
        #[cfg(target_os="android")]
        {
            // A per-session ID prevents a late permission callback from starting
            // a cancelled capture, or an old stop from killing a new session.
            let app = app.clone(); std::thread::spawn(move || { let _ = tauri_plugin_mobile_audio::stop(&app, generation); });
        }
    }
}
fn toggle_session(app: &AppHandle) {
    let state = app.state::<Controller>(); let inner = state.0.lock().unwrap();
    if inner.cancel.is_some() { drop(inner); stop_session(app); return; }
    let previous = inner.last.clone(); drop(inner);
    if let Some(options) = previous {
        if let Err(e) = start_session(app.clone(), options) { let _ = app.emit("session-notice", e.to_string()); }
    } else { let _ = app.emit("session-notice", "请先在控制面板填写 API Key 并启动一次"); }
}

#[tauri::command] fn list_audio_devices() -> Result<audio_router::AudioDevices, String> { audio_router::list_devices().map_err(|e| format!("{e:#}")) }
#[tauri::command] fn get_status(state: tauri::State<'_, Controller>) -> Status { state.0.lock().unwrap().status.clone() }
#[tauri::command] fn get_diagnostics(state: tauri::State<'_, LogStore>) -> Vec<LogEntry> { state.snapshot() }
#[tauri::command] fn clear_diagnostics(state: tauri::State<'_, LogStore>) -> u64 { state.clear() }
#[tauri::command]
async fn copy_diagnostic_text(app: AppHandle, text: String) -> Result<(), String> {
    if text.len() > 512 * 1024 { return Err("日志过长，请先清空后重新尝试".into()); }
    #[cfg(target_os="android")]
    { tauri_plugin_mobile_audio::copy_diagnostic_text(&app, &text).await }
    #[cfg(not(target_os="android"))]
    { let _ = (app, text); Err("桌面端请使用 WebView 剪贴板或手动复制".into()) }
}
#[tauri::command] fn start_translation(app: AppHandle, options: StartOptions) -> Result<(),String> { start_session(app, options).map_err(|e|format!("{e:#}")) }
#[tauri::command]
fn get_saved_settings(app: AppHandle, state: tauri::State<'_, Controller>) -> Result<Option<PersistedSettings>, String> {
    let saved = read_settings(&app).map_err(|e| format!("{e:#}"))?;
    if let Some(settings) = &saved {
        let transcription_only = settings.options.model.is_transcription_only();
        let overlay_scale = settings.options.overlay_scale;
        let mut inner = state.0.lock().unwrap();
        inner.last = Some(settings.options.clone());
        let mode_changed = inner.cancel.is_none()
            && matches!(inner.status.phase.as_str(), "idle" | "error")
            && inner.status.transcription_only != transcription_only;
        let scale_changed = (inner.status.overlay_scale - overlay_scale).abs() > f64::EPSILON;
        if mode_changed { inner.status.transcription_only = transcription_only; }
        if scale_changed { inner.status.overlay_scale = overlay_scale; }
        let status = if mode_changed || scale_changed { Some(inner.status.clone()) } else { None };
        drop(inner);
        resize_overlay(&app, overlay_scale).map_err(|e| format!("{e:#}"))?;
        if let Some(status) = status {
            publish(&app, &status);
            if mode_changed {
                let _ = app.emit("subtitle-update", Subtitle { original: String::new(), translated: String::new(), transcription_only });
            }
        }
    }
    Ok(saved)
}
#[tauri::command]
fn unlock_saved_secrets(app: AppHandle, state: tauri::State<'_, Controller>, password: String) -> Result<SecretPayload, String> {
    let payload = read_encrypted_secrets(&app, &password).map_err(|e| format!("{e:#}"))?;
    let mut inner = state.0.lock().unwrap();
    if let Some(options) = &mut inner.last {
        options.api_key = payload.api_key.clone();
        options.proxy_url = payload.proxy_url.clone();
    }
    Ok(payload)
}

// Keep the most recent controls in memory for F8 and persist non-sensitive
// preferences. Sensitive fields only enter the password-encrypted vault.
#[tauri::command]
fn remember_settings(app: AppHandle, state: tauri::State<'_, Controller>, mut options: StartOptions, save_sensitive: bool, encryption_password: String) -> Result<(), String> {
    options.overlay_scale = checked_overlay_scale(options.overlay_scale).map_err(|e| e.to_string())?;
    if save_sensitive {
        let vault_exists = config_path(&app, SECRETS_FILE).map_err(|e| format!("{e:#}"))?.exists();
        if encryption_password.is_empty() {
            if !vault_exists { return Err("请输入加密密码以保存 API Key".into()); }
        } else {
            let payload = SecretPayload { api_key: options.api_key.clone(), proxy_url: options.proxy_url.clone() };
            write_encrypted_secrets(&app, &encryption_password, &payload).map_err(|e| format!("{e:#}"))?;
        }
    } else {
        remove_encrypted_secrets(&app).map_err(|e| format!("{e:#}"))?;
    }
    let persisted = settings_for_storage(&options, save_sensitive);
    write_settings(&app, &persisted).map_err(|e| format!("{e:#}"))?;
    let transcription_only = options.model.is_transcription_only();
    let mut inner = state.0.lock().unwrap();
    // Editing ordinary preferences before unlocking must not erase secrets
    // already held in this process (for example after using F8).
    if save_sensitive && options.api_key.is_empty() {
        if let Some(previous) = &inner.last {
            options.api_key = previous.api_key.clone();
            if options.proxy_url.is_empty() { options.proxy_url = previous.proxy_url.clone(); }
        }
    }
    inner.last = Some(options);
    let changed = inner.cancel.is_none()
        && matches!(inner.status.phase.as_str(), "idle" | "error")
        && inner.status.transcription_only != transcription_only;
    if changed { inner.status.transcription_only = transcription_only; }
    let status = if changed { Some(inner.status.clone()) } else { None };
    drop(inner);
    if let Some(status) = status {
        publish(&app, &status);
        let _ = app.emit("subtitle-update", Subtitle { original: String::new(), translated: String::new(), transcription_only });
    }
    Ok(())
}
#[tauri::command] fn stop_translation(app: AppHandle) { stop_session(&app); }
#[tauri::command] fn toggle_translation(app: AppHandle) { toggle_session(&app); }
#[tauri::command]
fn set_overlay_scale(app: AppHandle, state: tauri::State<'_, Controller>, scale: f64) -> Result<(), String> {
    let scale = checked_overlay_scale(scale).map_err(|e| e.to_string())?;
    resize_overlay(&app, scale).map_err(|e| format!("{e:#}"))?;
    let mut inner = state.0.lock().unwrap();
    inner.status.overlay_scale = scale;
    if let Some(options) = &mut inner.last { options.overlay_scale = scale; }
    let status = inner.status.clone();
    drop(inner);
    publish(&app, &status);
    Ok(())
}
#[tauri::command]
async fn set_overlay_locked(app: AppHandle, locked: bool) -> Result<(), String> {
    #[cfg(not(target_os="android"))]
    {
        let window = app.get_webview_window("main").ok_or("字幕窗口不存在")?;
        window.set_ignore_cursor_events(locked).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os="android")]
    {
        let cloned = app.clone();
        tokio::task::spawn_blocking(move || tauri_plugin_mobile_audio::set_editable(&cloned, !locked)).await.map_err(|e| e.to_string())??;
    }
    let state = app.state::<Controller>(); let mut inner = state.0.lock().unwrap(); inner.status.overlay_locked = locked;
    let status = inner.status.clone(); drop(inner); publish(&app,&status); Ok(())
}
#[tauri::command]
async fn request_overlay_permission(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os="android")]
    { return tokio::task::spawn_blocking(move || tauri_plugin_mobile_audio::request_overlay(&app)).await.map_err(|e|e.to_string())?; }
    #[cfg(not(target_os="android"))] { let _ = app; Err("仅 Android 需要此权限".into()) }
}
#[tauri::command] fn quit_app(app: AppHandle) { stop_session(&app); app.exit(0); }

fn add_fragment(current: &mut String, incoming: &str, reset: &mut bool) {
    if *reset { current.clear(); *reset = false; }
    // Live backends may send either deltas or growing hypotheses. Treat an
    // exact repeated hypothesis as a duplicate, while accepting later deltas.
    if incoming == current && !current.is_empty() { return; }
    if incoming.starts_with(current.as_str()) && !current.is_empty() && incoming.len() > current.len() { *current = incoming.to_owned(); }
    else { current.push_str(incoming); }
    if current.chars().count() > 300 { *current = current.chars().rev().take(300).collect::<String>().chars().rev().collect(); }
}

// Live Transcribe sends a replaceable hypothesis and an authoritative final
// segment, not ordinary text deltas. Display the current segment only: never
// append a partial to its final or a new utterance to a previous hypothesis.
// When both fields arrive in the same event, the finalized text wins.
fn replace_transcript(current: &mut String, interim: Option<&str>, final_text: Option<&str>) -> bool {
    let Some(text) = final_text.or(interim) else { return false; };
    let text = if text.chars().count() > 300 {
        text.chars().rev().take(300).collect::<String>().chars().rev().collect()
    } else { text.to_owned() };
    if *current == text { return false; }
    *current = text;
    true
}

async fn session(app: &AppHandle, generation: u64, options: StartOptions, cancel: &mut watch::Receiver<bool>, trace: &Trace) -> Result<()> {
    let route_started = Instant::now();
    trace.info("开始选择系统网络 / 代理路径（不会修改全局设置）");
    let route = match proxy::resolve(app, options.proxy_mode, &options.proxy_url, cancel).await {
        Ok(route) => route,
        Err(e) => { trace.error(format!("选择网络路径失败（{} ms）：{}", route_started.elapsed().as_millis(), e)); return Err(e); },
    };
    if *cancel.borrow() { return Ok(()); }
    // This is the selected route, not a claim that the proxy or VPN works.
    set_network_route(app, generation, route.label());
    trace.info(format!("选定网络路径：{}，耗时 {} ms（不代表已连通）", route.label(), route_started.elapsed().as_millis()));
    let socket = gemini_client::connect(&options.gemini(), &route, cancel, trace).await?;
    let connected_at = tokio::time::Instant::now();
    if *cancel.borrow() { return Ok(()); }
    trace.info("Gemini 已初始化，正在准备音频采集和权限");
    let (pcm_tx, mut pcm_rx) = mpsc::channel::<Vec<u8>>(12);
    let (error_tx, mut error_rx) = mpsc::unbounded_channel::<String>();
    let mut router = AudioRouter::new(options.mode, pcm_tx);

    #[cfg(not(target_os="android"))]
    {
        let cfg = options.audio(); let errors = error_tx.clone();
        let started = tokio::task::spawn_blocking(move || -> Result<AudioRouter> { router.start_desktop(cfg, errors)?; Ok(router) }).await.context("音频采集任务异常").and_then(|r| r);
        router = match started {
            Ok(router) => router,
            Err(error) => { trace.error("音频采集准备失败；请查看界面状态提示"); return Err(error); },
        };
    }
    #[cfg(target_os="android")]
    let mut native_forwarder = {
        use audio_router::{RawAudio, Source};
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        use std::time::Duration;
        let (tx, rx) = crossbeam_channel::bounded(64);
        tauri_plugin_mobile_audio::set_sender(Some(tx));
        let raw = router.raw_sender.clone(); let err = error_tx.clone();
        let stop = Arc::new(AtomicBool::new(false)); let worker_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(tauri_plugin_mobile_audio::NativeMessage::Audio { system, sample_rate, samples }) => {
                        let data = samples.into_iter().map(|s| s as f32 / 32768.0).collect();
                        let _ = raw.try_send(RawAudio { source:if system { Source::System } else { Source::Mic }, sample_rate, channels:1, data });
                    }
                    Ok(tauri_plugin_mobile_audio::NativeMessage::Error(e)) => { let _ = err.send(e); }
                    Err(_) => {}
                }
            }
        });
        (stop, Some(thread))
    };

    #[cfg(target_os="android")]
    {
        let mode = match options.mode { AudioSourceMode::MicrophoneOnly => "microphone_only", AudioSourceMode::SystemAudioOnly => "system_audio_only", AudioSourceMode::Mixed => "mixed" };
        let started: Result<()> = tokio::select! {
            r = tokio::time::timeout(std::time::Duration::from_secs(120), tauri_plugin_mobile_audio::start(app, mode, generation, options.model.is_transcription_only())) => match r {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(anyhow!(e)),
                Err(_) => Err(anyhow!("等待 Android 权限或服务启动超时，请关闭授权窗口后重试")),
            },
            _ = gemini_client::cancelled(cancel) => Err(anyhow!("已停止")),
        };
        if let Err(error) = started {
            trace.warn("Android 音频采集或授权未完成；请查看界面状态提示");
            tauri_plugin_mobile_audio::set_sender(None);
            native_forwarder.0.store(true,std::sync::atomic::Ordering::Release);
            if let Some(h) = native_forwarder.1.take() { let _ = h.join(); }
            let cloned = app.clone(); let _ = tokio::task::spawn_blocking(move || tauri_plugin_mobile_audio::stop(&cloned, generation)).await;
            return Err(error);
        }
    }
    if *cancel.borrow() {
        #[cfg(target_os="android")] {
            tauri_plugin_mobile_audio::set_sender(None); native_forwarder.0.store(true,std::sync::atomic::Ordering::Release);
            if let Some(h)=native_forwarder.1.take() { let _=h.join(); }
            let a=app.clone(); let _=tokio::task::spawn_blocking(move ||tauri_plugin_mobile_audio::stop(&a, generation)).await;
        }
        return Ok(());
    }
    // Enforce text-only semantics even if an unexpected client sends a stale flag.
    let player = if options.play_translation() {
        let device = options.playback_device.clone(); let errors = error_tx.clone();
        match tokio::task::spawn_blocking(move || AudioPlayer::start(device, errors)).await {
            Ok(Ok(p)) => Some(p),
            Ok(Err(e)) => { let _ = app.emit("session-notice", format!("无法播放译音，字幕仍可用: {e:#}")); None },
            Err(e) => { let _ = app.emit("session-notice", format!("播放器异常: {e}")); None },
        }
    } else { None };

    #[cfg(target_os="android")]
    let (caption_tx, caption_task) = {
        // Keep only the most recent caption; a paused/destroyed Activity must
        // not block cleanup while a synchronous mobile-plugin call waits.
        let (tx, mut rx) = watch::channel::<Option<Subtitle>>(None); let handle = app.clone();
        let worker = tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let latest = rx.borrow_and_update().clone();
                if let Some(v) = latest {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), tauri_plugin_mobile_audio::caption(&handle, &v.original, &v.translated)).await;
                }
            }
        });
        (tx, worker)
    };
    // The router can emit silence while the device or Android consent dialog is
    // starting. Don't begin a session with up to 1.2 seconds of stale frames.
    while pcm_rx.try_recv().is_ok() {}
    trace.info("音频采集已准备，会话进入运行状态（不记录音频或字幕内容）");
    let transcription_only = options.model.is_transcription_only();
    set_phase(app, generation, "running", if transcription_only { "正在采集并转写原文" } else { "正在采集并翻译" });
    let (mut sink, mut stream) = socket.split();
    let mut caption = Subtitle { original: String::new(), translated: String::new(), transcription_only };
    let mut reset_input = false; let mut reset_output = false;
    let mut last_interaction_status = None;
    let mut seen_frames = HashSet::new();
    let (mut sent_audio, mut seen_input, mut seen_output, mut seen_audio) = (false, false, false, false);
    // Count from setupComplete, not from the end of an Android permission dialog.
    let expiry_warning = tokio::time::sleep_until(connected_at + std::time::Duration::from_secs(9 * 60));
    tokio::pin!(expiry_warning);
    let mut expiry_warned = false;
    let result: Result<()> = loop {
        tokio::select! {
            _ = &mut expiry_warning, if transcription_only && !expiry_warned => {
                expiry_warned = true;
                trace.warn("实时转写会话接近 10 分钟上限；未启用自动续接");
                let _ = app.emit("session-notice", "实时转写会话接近 10 分钟上限，断开后请手动重新开始。");
            }
            _ = gemini_client::cancelled(cancel) => {
                trace.info("收到停止请求，正在关闭音频流和 WebSocket");
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.send(gemini_client::stream_end())).await;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await;
                break Ok(());
            }
            Some(error) = error_rx.recv() => {
                trace.error("音频设备报告错误；请查看界面状态提示");
                break Err(anyhow!(error));
            },
            Some(frame) = pcm_rx.recv() => {
                let sent = tokio::select! {
                    _ = gemini_client::cancelled(cancel) => break Ok(()),
                    result = tokio::time::timeout(std::time::Duration::from_secs(8), sink.send(gemini_client::audio_frame(&frame))) => result,
                };
                match sent {
                    Ok(Ok(())) => if !sent_audio { trace.info("已发送首个音频帧（内容未记录）"); sent_audio = true; },
                    Ok(Err(e)) => { let message = format!("发送音频失败：{}", gemini_client::describe_ws_error(&e)); trace.error(&message); break Err(anyhow!(message)); },
                    Err(_) => { trace.error("发送音频超时；请检查网络"); break Err(anyhow!("发送音频超时；请检查网络")); },
                }
            }
            msg = stream.next() => {
                let message = match msg {
                    Some(Ok(v)) => v,
                    Some(Err(e)) => { let message = format!("Gemini 连接中断：{}", gemini_client::describe_ws_error(&e)); trace.error(&message); break Err(anyhow!(message)); },
                    None => { trace.warn("Gemini 连接已关闭（无关闭帧）"); break Err(anyhow!("Gemini 连接已关闭")); },
                };
                let (kind, value) = match gemini_client::decode_server_frame(message) {
                    Ok(ServerFrame::Data { kind, value }) => (kind, value),
                    Ok(ServerFrame::Close { code, hint }) => { let message = gemini_client::close_description(code, hint); trace.warn(&message); break Err(anyhow!(message)); },
                    Ok(ServerFrame::Control(_)) => continue,
                    Err(e) => { trace.error(format!("处理 Gemini 帧失败：{}", e)); break Err(e); },
                };
                let category = gemini_client::classify_message(&value);
                if seen_frames.insert((kind, category)) { trace.info(format!("会话收到 {kind} 帧，类型 {category}")); }
                let parsed = match gemini_client::parse_server_value(&value) {
                    Ok(p) => p,
                    Err(e) => { trace.error(format!("Gemini 消息处理失败：{}", e)); break Err(e); },
                };
                if (parsed.input.is_some() || parsed.interim_input.is_some()) && !seen_input { trace.info("已收到输入转写（内容未记录）"); seen_input = true; }
                if !transcription_only && parsed.output.is_some() && !seen_output { trace.info("已收到输出转写（内容未记录）"); seen_output = true; }
                if !transcription_only && !parsed.audio.is_empty() && !seen_audio { trace.info("已收到译音（内容未记录）"); seen_audio = true; }
                if parsed.go_away { trace.warn("Gemini 提示会话即将到期"); let _ = app.emit("session-notice", "Gemini 提示会话即将到期；连接结束后请重新启动"); }
                if options.model == ModelKind::ExtendedThinking {
                    if let Some(state) = parsed.interaction_status {
                        if last_interaction_status != Some(state) {
                            let text = match state {
                                InteractionStatus::InProgress => "正在采集并翻译 · 模型推理中",
                                InteractionStatus::Idle => "正在采集并翻译",
                            };
                            set_phase(app, generation, "running", text);
                            last_interaction_status = Some(state);
                        }
                    }
                }
                if parsed.interrupted { if let Some(p) = &player { p.clear(); } reset_output = true; }
                let mut changed = false;
                if transcription_only {
                    changed = replace_transcript(
                        &mut caption.original,
                        parsed.interim_input.as_ref().map(|(text, _)| text.as_str()),
                        parsed.input.as_ref().map(|(text, _)| text.as_str()),
                    );
                    // Neither output transcription nor audio is part of this mode.
                } else {
                    if let Some((text, finished)) = parsed.input { add_fragment(&mut caption.original, &text, &mut reset_input); if finished { reset_input=true; } changed = true; }
                    if let Some((text, finished)) = parsed.output { add_fragment(&mut caption.translated, &text, &mut reset_output); if finished { reset_output=true; } changed = true; }
                }
                if changed {
                    let _ = app.emit("subtitle-update", caption.clone());
                    #[cfg(target_os="android")] { let _ = caption_tx.send(Some(caption.clone())); }
                }
                if let Some(p) = &player { for bytes in parsed.audio { p.push(bytes); } }
                // Extended Thinking turnComplete is only a subtitle boundary;
                // it does not mean the whole connection has ended.
                if !transcription_only && parsed.turn_complete { reset_input=true; reset_output=true; }
            }
        }
    };
    #[cfg(target_os="android")]
    {
        drop(caption_tx); caption_task.abort();
        tauri_plugin_mobile_audio::set_sender(None); native_forwarder.0.store(true,std::sync::atomic::Ordering::Release);
        if let Some(h) = native_forwarder.1.take() { let _ = h.join(); }
        let handle = app.clone(); let _ = tokio::task::spawn_blocking(move || tauri_plugin_mobile_audio::stop(&handle, generation)).await;
    }
    router.stop(); drop(player);
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default().manage(Controller::default()).manage(LogStore::default());
    #[cfg(target_os="android")] { builder = builder.plugin(tauri_plugin_mobile_audio::init()); }
    builder = builder.invoke_handler(tauri::generate_handler![list_audio_devices,get_status,get_saved_settings,unlock_saved_secrets,get_diagnostics,clear_diagnostics,copy_diagnostic_text,start_translation,remember_settings,stop_translation,toggle_translation,set_overlay_scale,set_overlay_locked,request_overlay_permission,quit_app]);
    #[cfg(not(any(target_os="android", target_os="ios")))]
    {
        // The click-through overlay has no taskbar entry or tray. Closing the
        // control panel therefore exits rather than leaving an unreachable app.
        builder = builder.on_window_event(|window, event| {
            if window.label() == "control" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    stop_session(window.app_handle());
                    window.app_handle().exit(0);
                }
            }
        });
    }
    builder.setup(|app| {
        #[cfg(not(any(target_os="android", target_os="ios")))]
        {
            use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};
            let f8 = Shortcut::new(None, Code::F8);
            app.handle().plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(move |app, shortcut, event| {
                if shortcut == &f8 && event.state() == ShortcutState::Pressed { toggle_session(app); }
            }).build())?;
            let registered = app.global_shortcut().register(f8).is_ok();
            let state = app.state::<Controller>(); state.0.lock().unwrap().status.shortcut_available = registered;
            if let Some(window) = app.get_webview_window("main") { let _ = window.set_ignore_cursor_events(true); }
        }
        Ok(())
    }).run(tauri::generate_context!()).expect("Tauri 启动失败");
}

#[cfg(test)]
mod tests {
    use super::{add_fragment, checked_overlay_scale, decrypt_secrets, encrypt_secrets, replace_transcript, settings_for_storage, SecretPayload, StartOptions};

    #[test]
    fn subtitle_fragments_handle_deltas_hypotheses_and_reset() {
        let mut text = String::new(); let mut reset = false;
        add_fragment(&mut text, "你", &mut reset);
        add_fragment(&mut text, "你好", &mut reset); // growing hypothesis
        add_fragment(&mut text, "你好", &mut reset); // duplicate
        add_fragment(&mut text, "，世界", &mut reset); // delta
        assert_eq!(text, "你好，世界");
        reset = true; add_fragment(&mut text, "新的一句", &mut reset);
        assert_eq!(text, "新的一句");
        text = "甲".repeat(300); add_fragment(&mut text, "乙", &mut reset);
        assert_eq!(text.chars().count(), 300); assert!(text.ends_with('乙'));
    }

    #[test]
    fn live_transcription_replaces_hypotheses_and_authoritative_segments() {
        let mut text = String::new();
        assert!(replace_transcript(&mut text, Some("天气非"), None));
        assert!(replace_transcript(&mut text, Some("天气非常好"), None));
        assert_eq!(text, "天气非常好");
        assert!(replace_transcript(&mut text, Some("错误的猜测"), Some("天气很好。")));
        assert_eq!(text, "天气很好。");
        assert!(!replace_transcript(&mut text, None, Some("天气很好。")));
        assert!(replace_transcript(&mut text, Some("下一句"), None));
        assert_eq!(text, "下一句"); // no previous final or interim concatenation
        assert!(replace_transcript(&mut text, None, Some("下一句话。")));
        assert_eq!(text, "下一句话。");
        assert!(!replace_transcript(&mut text, None, None));
        assert!(replace_transcript(&mut text, Some(""), None));
        assert_eq!(text, "");
        let long = "中".repeat(301);
        replace_transcript(&mut text, None, Some(&long));
        assert_eq!(text.chars().count(), 300);
    }

    #[test]
    fn persisted_settings_require_opt_in_for_secrets() {
        let options: StartOptions = serde_json::from_value(serde_json::json!({
            "apiKey":"PRIVATE-KEY", "model":"general_live", "targetLanguage":"zh",
            "mode":"microphone_only", "proxyMode":"manual",
            "proxyUrl":"http://alice:password@127.0.0.1:7890"
        })).unwrap();
        let ordinary = settings_for_storage(&options, false);
        assert!(ordinary.options.api_key.is_empty());
        assert!(ordinary.options.proxy_url.is_empty());
        assert!(!ordinary.save_sensitive);
        let sensitive = settings_for_storage(&options, true);
        assert!(sensitive.options.api_key.is_empty());
        assert!(sensitive.options.proxy_url.is_empty());
        assert!(sensitive.save_sensitive);
        let json = serde_json::to_value(ordinary).unwrap();
        assert_eq!(json["options"]["model"], "general_live");
        assert_eq!(json["options"]["proxyMode"], "manual");
    }

    #[test]
    fn encrypted_secrets_round_trip_and_reject_wrong_password() {
        let payload = SecretPayload {
            api_key: "PRIVATE-KEY".into(),
            proxy_url: "socks5://alice:password@127.0.0.1:1080".into(),
        };
        let encrypted = encrypt_secrets("correct horse battery staple", &payload).unwrap();
        let text = String::from_utf8(encrypted.clone()).unwrap();
        assert!(!text.contains("PRIVATE-KEY"));
        assert!(!text.contains("alice:password"));
        let decrypted = decrypt_secrets("correct horse battery staple", &encrypted).unwrap();
        assert_eq!(decrypted.api_key, payload.api_key);
        assert_eq!(decrypted.proxy_url, payload.proxy_url);
        assert!(decrypt_secrets("wrong password", &encrypted).is_err());
        assert!(encrypt_secrets("short", &payload).is_err());
    }

    #[test]
    fn overlay_scale_is_bounded_and_normalized() {
        assert_eq!(checked_overlay_scale(1.26).unwrap(), 1.3);
        assert_eq!(checked_overlay_scale(0.8).unwrap(), 0.8);
        assert_eq!(checked_overlay_scale(1.6).unwrap(), 1.6);
        assert!(checked_overlay_scale(0.79).is_err());
        assert!(checked_overlay_scale(f64::NAN).is_err());
    }

    #[test]
    fn text_only_backend_rejects_stale_playback_or_bidirectional_flags() {
        let mut o: StartOptions = serde_json::from_value(serde_json::json!({
            "apiKey":"test", "model":"transcribe_live_35", "targetLanguage":"zh",
            "mode":"microphone_only", "playbackEnabled":true
        })).unwrap();
        assert!(o.validate().is_err());
        assert!(!o.play_translation()); // defense in depth even before validation
        o.playback_enabled = false;
        assert!(o.validate().is_ok());
        o.bidirectional = true;
        assert!(o.validate().is_err());
    }
}
