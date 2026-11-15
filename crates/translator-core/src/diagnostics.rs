//! Bounded, credential-safe diagnostics independent of UI and platform events.

use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::{
    cmp::Reverse,
    collections::VecDeque,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
use zeroize::Zeroize;

pub const MAX_DIAGNOSTIC_ENTRIES: usize = 300;
pub const MAX_DIAGNOSTIC_CHARS: usize = 1_200;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub session_id: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Default)]
struct Buffer {
    last_seq: u64,
    entries: VecDeque<LogEntry>,
}

#[derive(Default)]
pub struct DiagnosticStore(Mutex<Buffer>);

impl DiagnosticStore {
    /// SWR-DIAG-001: history is fixed at 300 entries and freshness-first.
    pub fn push(&self, session_id: u64, level: LogLevel, message: &str) -> LogEntry {
        let mut buffer = self.0.lock().unwrap_or_else(|error| error.into_inner());
        buffer.last_seq = buffer.last_seq.saturating_add(1);
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let entry = LogEntry {
            seq: buffer.last_seq,
            timestamp_ms,
            session_id,
            level,
            message: sanitize_message(message),
        };
        if buffer.entries.len() == MAX_DIAGNOSTIC_ENTRIES {
            buffer.entries.pop_front();
        }
        buffer.entries.push_back(entry.clone());
        entry
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .iter()
            .cloned()
            .collect()
    }

    /// Return a sequence watermark so delayed pre-clear events can be ignored.
    pub fn clear(&self) -> u64 {
        let mut buffer = self.0.lock().unwrap_or_else(|error| error.into_inner());
        buffer.entries.clear();
        buffer.last_seq
    }
}

pub struct Redactor {
    secrets: Vec<String>,
}

impl Drop for Redactor {
    fn drop(&mut self) {
        self.secrets.zeroize();
    }
}

impl Redactor {
    /// SWR-DIAG-002: derive all credential forms once at the trust boundary.
    pub fn new(api_key: &str, proxy_url: &str) -> Self {
        let mut value = Self {
            secrets: Vec::new(),
        };
        value.add_secret(api_key.trim());
        if let Ok(url) = Url::parse(proxy_url) {
            if !url.username().is_empty() || url.password().is_some() {
                value.add_secret(proxy_url.trim());
                value.add_secret(url.username());
                value.add_secret(&percent_decode_str(url.username()).decode_utf8_lossy());
                if let Some(password) = url.password() {
                    value.add_secret(password);
                    value.add_secret(&percent_decode_str(password).decode_utf8_lossy());
                }
            }
        }
        value
    }

    pub fn add_secret(&mut self, secret: &str) {
        if !secret.is_empty() && !self.secrets.iter().any(|value| value == secret) {
            self.secrets.push(secret.to_owned());
            self.secrets.sort_by_key(|value| Reverse(value.len()));
        }
    }

    /// SWR-DIAG-003: redact, remove controls and cap text before queueing.
    pub fn redact(&self, message: &str) -> String {
        let mut text = message.to_owned();
        for secret in &self.secrets {
            text = redact_secret(&text, secret);
        }
        sanitize_message(&text)
    }
}

fn sanitize_message(message: &str) -> String {
    let mut clean: String = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect();
    if message.chars().count() > MAX_DIAGNOSTIC_CHARS {
        clean.push_str("…[已截断]");
    }
    clean
}

fn redact_secret(input: &str, secret: &str) -> String {
    if secret.is_empty() {
        return input.to_owned();
    }
    if secret.chars().count() >= 4 {
        return input.replace(secret, "[已隐藏]");
    }
    let mut output = String::new();
    let mut last = 0;
    for (start, _) in input.match_indices(secret) {
        let end = start + secret.len();
        let is_word = |character: char| character.is_alphanumeric() || character == '_';
        let left = input[..start].chars().next_back().is_some_and(is_word);
        let right = input[end..].chars().next().is_some_and(is_word);
        if !left && !right {
            output.push_str(&input[last..start]);
            output.push_str("[已隐藏]");
            last = end;
        }
    }
    output.push_str(&input[last..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_diag_001_store_is_bounded_monotonic_and_clearable() {
        let store = DiagnosticStore::default();
        for index in 0..305 {
            store.push(1, LogLevel::Info, &index.to_string());
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.len(), MAX_DIAGNOSTIC_ENTRIES);
        assert_eq!(snapshot[0].seq, 6);
        assert_eq!(store.clear(), 305);
        assert!(store.snapshot().is_empty());
        assert_eq!(store.push(2, LogLevel::Warn, "after").seq, 306);
    }

    #[test]
    fn swr_diag_002_redactor_masks_encoded_and_decoded_credentials() {
        let redactor = Redactor::new(
            "AIza-fake-secret-123",
            "http://alice:p%40ssword@127.0.0.1:7890",
        );
        let text =
            redactor.redact("server AIza-fake-secret-123 alice p@ssword p%40ssword\nnext line");
        for secret in ["AIza-fake-secret-123", "alice", "p@ssword", "p%40ssword"] {
            assert!(!text.contains(secret));
        }
        assert!(!text.contains('\n'));
        assert!(text.contains("[已隐藏]"));
    }

    #[test]
    fn swr_diag_003_messages_are_bounded_and_short_tokens_do_not_garble_words() {
        assert_eq!(
            redact_secret("setupComplete password=p", "p"),
            "setupComplete password=[已隐藏]"
        );
        assert_eq!(
            redact_secret("user=u@127.0.0.1", "u"),
            "user=[已隐藏]@127.0.0.1"
        );
        let store = DiagnosticStore::default();
        let entry = store.push(1, LogLevel::Error, &"x".repeat(MAX_DIAGNOSTIC_CHARS + 50));
        assert_eq!(entry.message.chars().count(), MAX_DIAGNOSTIC_CHARS + 6);
        assert!(entry.message.ends_with("…[已截断]"));
    }

    #[test]
    fn swr_diag_004_level_serialization_remains_frontend_compatible() {
        let entry = DiagnosticStore::default().push(7, LogLevel::Error, "safe");
        let value = serde_json::to_value(entry).expect("serializable entry");
        assert_eq!(value["level"], "ERROR");
        assert_eq!(value["sessionId"], 7);
    }
}
