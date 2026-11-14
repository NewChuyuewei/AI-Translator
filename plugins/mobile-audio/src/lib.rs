//! Android native audio bridge. PCM never travels through WebView JSON IPC.
use crossbeam_channel::Sender;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use tauri::{plugin::{Builder, PluginHandle, TauriPlugin}, AppHandle, Manager, Runtime};

pub enum NativeMessage {
    Audio { system: bool, sample_rate: u32, samples: Vec<i16> },
    Error(String),
}
static SENDER: OnceLock<Mutex<Option<Sender<NativeMessage>>>> = OnceLock::new();
pub fn set_sender(sender: Option<Sender<NativeMessage>>) {
    if let Ok(mut s) = SENDER.get_or_init(|| Mutex::new(None)).lock() { *s = sender; }
}
fn send(message: NativeMessage) {
    if let Some(lock) = SENDER.get() { if let Ok(s) = lock.lock() { if let Some(tx) = s.as_ref() { let _ = tx.try_send(message); } } }
}

struct MobileAudio<R: Runtime>(PluginHandle<R>);
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mobile-audio")
        .setup(|app, api| {
            let handle = api.register_android_plugin("com.gemini.livetranslator.audio", "AudioPlugin")?;
            app.manage(MobileAudio(handle)); Ok(())
        }).build()
}
fn call<R: Runtime>(app: &AppHandle<R>, method: &str, value: Value) -> Result<Value, String> {
    app.state::<MobileAudio<R>>().0.run_mobile_plugin(method, value).map_err(|e| e.to_string())
}
/// Await permission/projection asynchronously. Dropping the future does not
/// block a Rust worker while an Android dialog remains open.
pub async fn start<R: Runtime>(app: &AppHandle<R>, mode: &str, session_id: u64, transcription_only: bool) -> Result<(), String> {
    let _: Value = app.state::<MobileAudio<R>>().0
        .run_mobile_plugin_async("startCapture", json!({"mode":mode, "sessionId":session_id, "transcriptionOnly":transcription_only}))
        .await.map_err(|e| e.to_string())?;
    Ok(())
}
pub fn stop<R: Runtime>(app: &AppHandle<R>, session_id: u64) -> Result<(), String> {
    call(app,"stopCapture",json!({"sessionId":session_id})).map(|_| ())
}
pub fn request_overlay<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> { call(app,"requestOverlay",json!({})).map(|_| ()) }
pub fn set_editable<R: Runtime>(app: &AppHandle<R>, editable: bool) -> Result<(), String> { call(app,"setOverlayEditable",json!({"editable":editable})).map(|_| ()) }
pub async fn caption<R: Runtime>(app: &AppHandle<R>, input: &str, output: &str) -> Result<(), String> {
    let _: Value = app.state::<MobileAudio<R>>().0
        .run_mobile_plugin_async("caption", json!({"input":input,"output":output}))
        .await.map_err(|e| e.to_string())?;
    Ok(())
}
/// Ask Android's ProxySelector about this HTTPS destination. The plugin
/// returns its first selected route only; it never changes global settings.
pub async fn system_proxy<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<Value, String> {
    app.state::<MobileAudio<R>>().0
        .run_mobile_plugin_async("systemProxy", json!({"url":url}))
        .await.map_err(|e| e.to_string())
}
/// Android's native clipboard works even when the WebView is not a secure
/// origin and navigator.clipboard is therefore unavailable.
pub async fn copy_diagnostic_text<R: Runtime>(app: &AppHandle<R>, text: &str) -> Result<(), String> {
    let _: Value = app.state::<MobileAudio<R>>().0
        .run_mobile_plugin_async("copyDiagnosticText", json!({"text": text}))
        .await.map_err(|_| "Android 剪贴板不可用；请手动选择日志并复制".to_owned())?;
    Ok(())
}

#[cfg(target_os = "android")]
mod native {
    use super::*;
    use jni::{objects::{JClass, JShortArray, JString}, sys::{jint, jboolean}, JNIEnv};
    #[no_mangle]
    pub extern "system" fn Java_com_gemini_livetranslator_audio_NativeAudioBridge_pushAudio(
        mut env: JNIEnv, _class: JClass, system: jboolean, data: JShortArray, length: jint, sample_rate: jint,
    ) {
        if length <= 0 || length > 32_000 || sample_rate <= 0 { return; }
        if let Ok(actual) = env.get_array_length(&data) {
            let n = length.min(actual) as usize; let mut samples = vec![0i16; n];
            if env.get_short_array_region(&data, 0, &mut samples).is_ok() {
                send(NativeMessage::Audio { system: system != 0, sample_rate: sample_rate as u32, samples });
            }
        }
    }
    #[no_mangle]
    pub extern "system" fn Java_com_gemini_livetranslator_audio_NativeAudioBridge_reportError(
        mut env: JNIEnv, _class: JClass, message: JString,
    ) {
        if let Ok(text) = env.get_string(&message) { send(NativeMessage::Error(text.into())); }
    }
}
