//! Slint adapter for bounded core diagnostics. Credentials are registered at
//! each network/session trust boundary before any related status is recorded.

use anyhow::{Result, anyhow};
use std::{
    io::Write,
    process::{Command, Stdio},
};
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

    pub fn copy_to_clipboard(&self) -> Result<()> {
        copy_platform_text(&self.copy_text())
    }

    pub fn clear(&self) -> u64 {
        self.store.clear()
    }
}

#[cfg(target_os = "windows")]
fn copy_platform_text(text: &str) -> Result<()> {
    pipe_text(Command::new("cmd").args(["/C", "clip"]), text)
}

#[cfg(target_os = "macos")]
fn copy_platform_text(text: &str) -> Result<()> {
    pipe_text(&mut Command::new("pbcopy"), text)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn copy_platform_text(text: &str) -> Result<()> {
    if pipe_text(&mut Command::new("wl-copy"), text).is_ok() {
        return Ok(());
    }
    pipe_text(
        Command::new("xclip").args(["-selection", "clipboard"]),
        text,
    )
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn copy_platform_text(_text: &str) -> Result<()> {
    Err(anyhow!("当前平台不支持诊断复制"))
}

fn pipe_text(command: &mut Command, text: &str) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| anyhow!("系统剪贴板不可用"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("系统剪贴板不可用"))?
        .write_all(text.as_bytes())
        .map_err(|_| anyhow!("系统剪贴板写入失败"))?;
    let status = child.wait().map_err(|_| anyhow!("系统剪贴板写入失败"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("系统剪贴板写入失败"))
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
