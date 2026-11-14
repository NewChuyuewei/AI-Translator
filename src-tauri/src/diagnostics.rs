//! Bounded in-memory diagnostics. Never record API keys, proxy credentials,
//! audio payloads, transcriptions, or complete request/response bodies.
use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::{collections::VecDeque, io::Write, sync::Mutex, time::{SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager};
use url::Url;

const MAX_ENTRIES: usize = 300;
const MAX_MESSAGE_CHARS: usize = 1200;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub session_id: u64,
    pub level: &'static str,
    pub message: String,
}
#[derive(Default)]
struct Buffer { last_seq: u64, entries: VecDeque<LogEntry> }
#[derive(Default)]
pub struct LogStore(Mutex<Buffer>);
impl LogStore {
    fn push(&self, session_id: u64, level: &'static str, message: String) -> LogEntry {
        let mut buffer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        buffer.last_seq = buffer.last_seq.saturating_add(1);
        let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64;
        let entry = LogEntry { seq: buffer.last_seq, timestamp_ms, session_id, level, message };
        if buffer.entries.len() >= MAX_ENTRIES { buffer.entries.pop_front(); }
        buffer.entries.push_back(entry.clone());
        entry
    }
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).entries.iter().cloned().collect()
    }
    /// Returns a sequence watermark so the UI can ignore delayed pre-clear events.
    pub fn clear(&self) -> u64 {
        let mut buffer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        buffer.entries.clear();
        buffer.last_seq
    }
}

// Short credentials are redacted at word boundaries, so a one-character
// password doesn't corrupt every occurrence of that character in normal logs.
fn redact_secret(input: &str, secret: &str) -> String {
    if secret.is_empty() { return input.to_owned(); }
    if secret.chars().count() >= 4 { return input.replace(secret, "[已隐藏]"); }
    let mut out = String::new();
    let mut last = 0;
    for (start, _) in input.match_indices(secret) {
        let end = start + secret.len();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let left = input[..start].chars().next_back().is_some_and(is_word);
        let right = input[end..].chars().next().is_some_and(is_word);
        if !left && !right {
            out.push_str(&input[last..start]); out.push_str("[已隐藏]"); last = end;
        }
    }
    out.push_str(&input[last..]); out
}

pub struct Trace {
    app: Option<AppHandle>,
    session_id: u64,
    secrets: Vec<String>,
    #[cfg(test)] capture: Option<std::sync::Arc<Mutex<Vec<String>>>>,
}
impl Trace {
    fn make(app: Option<AppHandle>, session_id: u64, api_key: &str, proxy_url: &str) -> Self {
        let mut trace = Self { app, session_id, secrets: Vec::new(), #[cfg(test)] capture: None };
        trace.add_secret(api_key.trim());
        if let Ok(url) = Url::parse(proxy_url) {
            if !url.username().is_empty() || url.password().is_some() {
                trace.add_secret(proxy_url.trim());
                trace.add_secret(url.username());
                trace.add_secret(&percent_decode_str(url.username()).decode_utf8_lossy());
                if let Some(password) = url.password() {
                    trace.add_secret(password);
                    trace.add_secret(&percent_decode_str(password).decode_utf8_lossy());
                }
            }
        }
        trace
    }
    pub fn for_app(app: AppHandle, session_id: u64, api_key: &str, proxy_url: &str) -> Self {
        Self::make(Some(app), session_id, api_key, proxy_url)
    }
    #[cfg(test)]
    pub fn silent() -> Self { Self::make(None, 0, "", "") }
    #[cfg(test)]
    pub fn capturing(api_key: &str, proxy_url: &str) -> (Self, std::sync::Arc<Mutex<Vec<String>>>) {
        let output = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut trace = Self::make(None, 1, api_key, proxy_url);
        trace.capture = Some(output.clone()); (trace, output)
    }
    pub fn add_secret(&mut self, secret: &str) {
        if !secret.is_empty() && !self.secrets.iter().any(|s| s == secret) {
            self.secrets.push(secret.to_owned());
            // Mask full URLs / long secrets before masking their substrings.
            self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }
    pub fn redact(&self, message: &str) -> String {
        let mut text = message.to_owned();
        for secret in &self.secrets { text = redact_secret(&text, secret); }
        let mut clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_MESSAGE_CHARS).collect();
        if text.chars().count() > MAX_MESSAGE_CHARS { clean.push_str("…[已截断]"); }
        clean
    }
    fn write(&self, level: &'static str, message: &str) {
        let safe = self.redact(message);
        #[cfg(test)]
        if let Some(output) = &self.capture { output.lock().unwrap().push(format!("{level}: {safe}")); }
        if let Some(app) = &self.app {
            let entry = app.state::<LogStore>().push(self.session_id, level, safe);
            // Available when launched from a terminal/logcat; the in-app console
            // works even for Windows GUI processes without an attached terminal.
            let _ = writeln!(std::io::stderr().lock(), "[Gemini #{}/{}] {}", entry.session_id, entry.level, entry.message);
            let _ = app.emit("diagnostic-log", entry);
        }
    }
    pub fn info(&self, message: impl AsRef<str>) { self.write("INFO", message.as_ref()); }
    pub fn warn(&self, message: impl AsRef<str>) { self.write("WARN", message.as_ref()); }
    pub fn error(&self, message: impl AsRef<str>) { self.write("ERROR", message.as_ref()); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_known_secrets_and_bounds_log_history() {
        let (trace, captured) = Trace::capturing("AIza-fake-secret-123", "http://alice:p%40ssword@127.0.0.1:7890");
        trace.error("server: AIza-fake-secret-123 alice p@ssword p%40ssword\nnext line");
        let text = captured.lock().unwrap().join("\n");
        for secret in ["AIza-fake-secret-123", "alice", "p@ssword", "p%40ssword"] { assert!(!text.contains(secret)); }
        assert!(!text.contains('\n')); assert!(text.contains("[已隐藏]"));
        let store = LogStore::default();
        for i in 0..305 { store.push(1, "INFO", i.to_string()); }
        assert_eq!(store.snapshot().len(), 300);
        assert_eq!(store.snapshot()[0].seq, 6);
        assert_eq!(store.clear(), 305);
        assert!(store.snapshot().is_empty());
        assert_eq!(store.push(2,"INFO","after".into()).seq, 306);
    }
    #[test]
    fn short_secrets_are_masked_as_tokens_without_garbling_static_text() {
        assert_eq!(redact_secret("setupComplete password=p", "p"), "setupComplete password=[已隐藏]");
        assert_eq!(redact_secret("user=u@127.0.0.1", "u"), "user=[已隐藏]@127.0.0.1");
    }
}
