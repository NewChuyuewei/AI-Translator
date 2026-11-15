//! Slint adapter for bounded core diagnostics. Credentials are registered at
//! each network/session trust boundary before any related status is recorded.

use translator_core::diagnostics::{DiagnosticStore, LogLevel, Redactor};

const MAX_COPY_CHARS: usize = 64 * 1024;

#[derive(Default)]
pub struct NativeDiagnostics {
    store: DiagnosticStore,
    redactor: Option<Redactor>,
    session_id: u64,
}

impl NativeDiagnostics {
    pub fn begin_boundary(&mut self, api_key: &str, proxy_url: &str, label: &str) {
        self.session_id = self.session_id.saturating_add(1);
        self.redactor = Some(Redactor::new(api_key, proxy_url));
        self.record(LogLevel::Info, label);
    }

    pub fn record(&self, level: LogLevel, message: &str) {
        let safe = self
            .redactor
            .as_ref()
            .map_or_else(|| message.to_owned(), |redactor| redactor.redact(message));
        self.store.push(self.session_id, level, &safe);
    }

    pub fn lines(&self) -> Vec<String> {
        self.store
            .snapshot()
            .into_iter()
            .map(|entry| {
                format!(
                    "#{:04} [{}] {}",
                    entry.seq,
                    entry.level.as_str(),
                    entry.message
                )
            })
            .collect()
    }

    pub fn copy_text(&self) -> String {
        let mut output = String::new();
        for line in self.lines().into_iter().rev() {
            let separator = usize::from(!output.is_empty());
            let remaining = MAX_COPY_CHARS.saturating_sub(output.chars().count());
            if remaining <= separator {
                break;
            }
            let available = remaining - separator;
            let clipped: String = line.chars().take(available).collect();
            if clipped.is_empty() {
                break;
            }
            if !output.is_empty() {
                output.insert(0, '\n');
            }
            output.insert_str(0, &clipped);
            if clipped.chars().count() < line.chars().count() {
                break;
            }
        }
        output
    }

    pub fn clear(&self) -> u64 {
        self.store.clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_ndiag_001_boundary_redacts_ui_lines_and_copy_text() {
        let mut diagnostics = NativeDiagnostics::default();
        diagnostics.begin_boundary(
            "PRIVATE-KEY-123",
            "http://user:p%40ss@127.0.0.1:7890",
            "开始会话",
        );
        diagnostics.record(
            LogLevel::Error,
            "PRIVATE-KEY-123 user p@ss p%40ss must not escape",
        );
        let text = diagnostics.copy_text();
        for secret in ["PRIVATE-KEY-123", "user", "p@ss", "p%40ss"] {
            assert!(!text.contains(secret));
        }
        assert!(text.contains("[已隐藏]"));
    }

    #[test]
    fn swr_ndiag_002_copy_is_bounded_and_clear_uses_core_watermark() {
        let diagnostics = NativeDiagnostics::default();
        for _ in 0..300 {
            diagnostics.record(LogLevel::Info, &"界".repeat(1_200));
        }
        assert!(diagnostics.copy_text().chars().count() <= MAX_COPY_CHARS);
        assert_eq!(diagnostics.lines().len(), 300);
        assert_eq!(diagnostics.clear(), 300);
        assert!(diagnostics.lines().is_empty());
    }
}
