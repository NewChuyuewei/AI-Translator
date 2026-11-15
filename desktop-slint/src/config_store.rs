//! Native desktop persistence adapter for ordinary settings and the encrypted vault.

use anyhow::{Context, Result, anyhow};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use translator_core::{
    SessionOptions,
    settings::{
        MAX_SETTINGS_BYTES, PersistedSettings, decode_settings, encode_settings,
        settings_for_storage,
    },
    vault::{MAX_VAULT_BYTES, SecretPayload, decrypt_secrets, encrypt_secrets},
};

const APP_IDENTIFIER: &str = "com.gemini.live.translator";
const SETTINGS_FILE: &str = "settings.json";
const VAULT_FILE: &str = "secrets.enc.json";

pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    /// SWR-NCFG-001: resolve the same identifier-based directory used by the
    /// legacy desktop without silently falling back to the working directory.
    pub fn discover() -> Result<Self> {
        let base = platform_config_base()?;
        Ok(Self::at(base.join(APP_IDENTIFIER)))
    }

    fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn load_settings(&self) -> Result<Option<PersistedSettings>> {
        let path = self.root.join(SETTINGS_FILE);
        if !path.try_exists().context("无法检查用户设置")? {
            return Ok(None);
        }
        let bytes = read_bounded(&path, MAX_SETTINGS_BYTES, "用户设置")?;
        let decoded = decode_settings(&bytes)?;
        if decoded.migration_required {
            self.write_settings(&decoded.settings)
                .context("无法迁移旧用户设置")?;
        }
        Ok(Some(decoded.settings))
    }

    pub fn save_settings(&self, options: &SessionOptions, save_sensitive: bool) -> Result<()> {
        self.write_settings(&settings_for_storage(options, save_sensitive))
    }

    fn write_settings(&self, settings: &PersistedSettings) -> Result<()> {
        let bytes = encode_settings(settings)?;
        write_private(&self.root.join(SETTINGS_FILE), &bytes)
    }

    pub fn vault_exists(&self) -> Result<bool> {
        self.root
            .join(VAULT_FILE)
            .try_exists()
            .context("无法检查加密配置")
    }

    pub fn unlock_vault(&self, password: &str) -> Result<SecretPayload> {
        let path = self.root.join(VAULT_FILE);
        if !path.try_exists().context("无法检查加密配置")? {
            return Err(anyhow!("没有找到加密配置；请填写 API Key 后重新保存"));
        }
        let bytes = read_bounded(&path, MAX_VAULT_BYTES, "加密配置")?;
        decrypt_secrets(password, &bytes)
    }

    pub fn save_vault(&self, password: &str, payload: &SecretPayload) -> Result<()> {
        let bytes = encrypt_secrets(password, payload)?;
        write_private(&self.root.join(VAULT_FILE), &bytes)
    }

    pub fn remove_vault(&self) -> Result<()> {
        match fs::remove_file(self.root.join(VAULT_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("无法删除加密配置"),
        }
    }
}

fn platform_config_base() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let value = env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let value = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let value = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        });
    #[cfg(not(any(windows, unix)))]
    let value: Option<PathBuf> = None;

    value.ok_or_else(|| anyhow!("无法定位应用配置目录"))
}

fn read_bounded(path: &Path, limit: usize, description: &str) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("无法读取{description}"))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("无法读取{description}"))?;
    if bytes.len() > limit {
        return Err(anyhow!("{description}超过 1 MiB 安全限制"));
    }
    Ok(bytes)
}

/// SWR-NCFG-002: durable same-directory replacement; Windows retains a
/// recoverable backup until replacement succeeds.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("应用配置目录无效"))?;
    fs::create_dir_all(parent).context("无法创建应用配置目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .context("无法限制应用配置目录权限")?;
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
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .context("无法限制配置文件权限")?;
    }

    #[cfg(not(windows))]
    fs::rename(&temporary, path).context("无法原子保存应用配置")?;
    #[cfg(windows)]
    replace_windows(path, &temporary)?;

    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .context("无法同步应用配置目录")?;
    Ok(())
}

#[cfg(windows)]
fn replace_windows(path: &Path, temporary: &Path) -> Result<()> {
    let backup = path.with_extension("bak");
    if backup.exists() {
        fs::remove_file(&backup).context("无法清理旧配置备份")?;
    }
    let had_previous = path.exists();
    if had_previous {
        fs::rename(path, &backup).context("无法备份旧应用配置")?;
    }
    if let Err(error) = fs::rename(temporary, path) {
        if had_previous {
            let _ = fs::rename(&backup, path);
        }
        return Err(error).context("无法保存新的应用配置");
    }
    if had_previous {
        fs::remove_file(&backup).context("无法清理应用配置备份")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use translator_core::{
        AudioSourceMode, ModelKind, ProxyMode, ThinkingLevel, TranscriptionMode,
    };

    fn fixture_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!("translator-{name}-{}-{nonce}", std::process::id()))
    }

    fn options() -> SessionOptions {
        SessionOptions {
            api_key: "PRIVATE-KEY".into(),
            model: ModelKind::GeneralLive,
            target_language: "it".into(),
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            bidirectional: false,
            other_language: "en".into(),
            mode: AudioSourceMode::MicrophoneOnly,
            microphone_device: None,
            system_device: None,
            playback_enabled: false,
            playback_device: None,
            token_saver: true,
            proxy_mode: ProxyMode::Manual,
            proxy_url: "http://alice:password@127.0.0.1:7890".into(),
            overlay_scale: 1.2,
        }
    }

    #[test]
    fn swr_ncfg_001_store_round_trips_settings_and_vault_separately() {
        let root = fixture_root("round-trip");
        let store = ConfigStore::at(root.clone());
        let options = options();
        store.save_settings(&options, true).expect("save settings");
        let saved = store
            .load_settings()
            .expect("load settings")
            .expect("present");
        assert!(saved.options.api_key.is_empty());
        assert!(saved.options.proxy_url.is_empty());
        assert!(saved.save_sensitive);

        let payload = SecretPayload {
            api_key: options.api_key.clone(),
            proxy_url: options.proxy_url.clone(),
        };
        store
            .save_vault("correct horse battery staple", &payload)
            .expect("save vault");
        assert!(store.vault_exists().expect("vault check"));
        assert_eq!(
            store
                .unlock_vault("correct horse battery staple")
                .expect("unlock"),
            payload
        );
        store.remove_vault().expect("remove vault");
        assert!(!store.vault_exists().expect("vault check"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn swr_ncfg_002_replacement_does_not_leave_plaintext_or_temp_files() {
        let root = fixture_root("replace");
        let store = ConfigStore::at(root.clone());
        store.save_settings(&options(), false).expect("first save");
        let mut changed = options();
        changed.target_language = "ja".into();
        store
            .save_settings(&changed, false)
            .expect("replacement save");
        let bytes = fs::read(root.join(SETTINGS_FILE)).expect("read settings");
        let text = String::from_utf8(bytes).expect("settings UTF-8");
        assert!(!text.contains("PRIVATE-KEY"));
        assert!(!text.contains("alice"));
        assert!(!root.join("settings.tmp").exists());
        assert_eq!(
            store
                .load_settings()
                .expect("load")
                .expect("present")
                .options
                .target_language,
            "ja"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn swr_ncfg_003_unix_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture_root("permissions");
        let store = ConfigStore::at(root.clone());
        store
            .save_settings(&options(), false)
            .expect("save settings");
        let directory_mode = fs::metadata(&root)
            .expect("directory metadata")
            .permissions()
            .mode();
        let file_mode = fs::metadata(root.join(SETTINGS_FILE))
            .expect("file metadata")
            .permissions()
            .mode();
        assert_eq!(directory_mode & 0o777, 0o700);
        assert_eq!(file_mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(root);
    }
}
