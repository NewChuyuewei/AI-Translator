//! Tauri event adapter for the bounded, credential-safe core diagnostics.
use std::io::Write;
#[cfg(test)]
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use translator_core::diagnostics::{LogLevel, Redactor};

pub use translator_core::diagnostics::{DiagnosticStore as LogStore, LogEntry};

pub struct Trace {
    app: Option<AppHandle>,
    session_id: u64,
    redactor: Redactor,
    #[cfg(test)]
    capture: Option<std::sync::Arc<Mutex<Vec<String>>>>,
}

impl Trace {
    fn make(app: Option<AppHandle>, session_id: u64, api_key: &str, proxy_url: &str) -> Self {
        Self {
            app,
            session_id,
            redactor: Redactor::new(api_key, proxy_url),
            #[cfg(test)]
            capture: None,
        }
    }

    pub fn for_app(app: AppHandle, session_id: u64, api_key: &str, proxy_url: &str) -> Self {
        Self::make(Some(app), session_id, api_key, proxy_url)
    }

    #[cfg(test)]
    pub fn silent() -> Self {
        Self::make(None, 0, "", "")
    }

    #[cfg(test)]
    pub fn capturing(api_key: &str, proxy_url: &str) -> (Self, std::sync::Arc<Mutex<Vec<String>>>) {
        let output = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut trace = Self::make(None, 1, api_key, proxy_url);
        trace.capture = Some(output.clone());
        (trace, output)
    }

    pub fn add_secret(&mut self, secret: &str) {
        self.redactor.add_secret(secret);
    }

    pub fn redact(&self, message: &str) -> String {
        self.redactor.redact(message)
    }

    fn write(&self, level: LogLevel, message: &str) {
        let safe = self.redactor.redact(message);
        #[cfg(test)]
        if let Some(output) = &self.capture {
            output
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(format!("{}: {safe}", level.as_str()));
        }
        if let Some(app) = &self.app {
            let entry = app.state::<LogStore>().push(self.session_id, level, &safe);
            let _ = writeln!(
                std::io::stderr().lock(),
                "[Gemini #{}/{}] {}",
                entry.session_id,
                entry.level.as_str(),
                entry.message
            );
            let _ = app.emit("diagnostic-log", entry);
        }
    }

    pub fn info(&self, message: impl AsRef<str>) {
        self.write(LogLevel::Info, message.as_ref());
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        self.write(LogLevel::Warn, message.as_ref());
    }

    pub fn error(&self, message: impl AsRef<str>) {
        self.write(LogLevel::Error, message.as_ref());
    }
}
