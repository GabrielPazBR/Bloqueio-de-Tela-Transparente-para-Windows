use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::SaltString;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub const CONFIG_VERSION: u32 = 1;
pub const DEFAULT_UNLOCK_MESSAGE: &str = "Digite a senha para desbloquear";
pub const MAX_UNLOCK_MESSAGE_CHARS: usize = 80;
pub const DEFAULT_DIMMING_PERCENTAGE: u8 = 40;
pub const IDLE_TIMEOUT_OPTIONS_MINUTES: [u16; 7] = [0, 1, 5, 10, 15, 30, 60];

pub fn default_unlock_message() -> String {
    DEFAULT_UNLOCK_MESSAGE.into()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    #[default]
    None,
    Clock,
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetConfig {
    pub kind: WidgetKind,
    pub image_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub x_percent: u8,
    pub y_percent: u8,
    #[serde(default = "default_widget_opacity")]
    pub opacity_percentage: u8,
}

pub const fn default_widget_opacity() -> u8 {
    0
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self {
            kind: WidgetKind::None,
            image_path: None,
            width: 400,
            height: 120,
            x_percent: 50,
            y_percent: 5,
            opacity_percentage: default_widget_opacity(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hotkey {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            control: true,
            alt: false,
            shift: true,
            key: "L".to_owned(),
        }
    }
}

impl Hotkey {
    pub fn display_name(&self) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub windows_hello_enabled: bool,
    #[serde(default)]
    pub idle_timeout_minutes: u16,
    #[serde(default)]
    pub win_l_enabled: bool,
    #[serde(default)]
    pub dimming_percentage: u8,
    #[serde(default = "default_unlock_message")]
    pub unlock_message: String,
    #[serde(default)]
    pub hide_taskbar_on_lock: bool,
    #[serde(default)]
    pub widget: WidgetConfig,
    #[serde(default)]
    pub unlock_logo_path: Option<String>,
    pub hotkey: Hotkey,
    pub password_hash: String,
}

impl AppConfig {
    pub fn for_test() -> Self {
        Self {
            version: CONFIG_VERSION,
            enabled: true,
            windows_hello_enabled: false,
            idle_timeout_minutes: 0,
            win_l_enabled: false,
            dimming_percentage: 0,
            unlock_message: default_unlock_message(),
            hide_taskbar_on_lock: false,
            widget: WidgetConfig::default(),
            unlock_logo_path: None,
            hotkey: Hotkey::default(),
            password_hash: String::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("a senha deve ter no máximo 128 caracteres")]
    InvalidPasswordLength,
    #[error("senha atual incorreta")]
    AuthenticationFailed,
    #[error("hash de senha corrompido")]
    CorruptPasswordHash,
    #[error("versão de configuração incompatível")]
    UnsupportedVersion,
    #[error("erro de entrada ou saída: {0}")]
    Io(String),
    #[error("configuração inválida: {0}")]
    InvalidConfig(String),
}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockPasswordResult {
    Accepted,
    Rejected,
    DisabledByWindowsHello,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn initialize(&self, password: &str, hotkey: Hotkey) -> Result<(), ConfigError> {
        validate_password_length(password)?;
        let config = AppConfig {
            version: CONFIG_VERSION,
            enabled: true,
            windows_hello_enabled: false,
            idle_timeout_minutes: 0,
            win_l_enabled: false,
            dimming_percentage: DEFAULT_DIMMING_PERCENTAGE,
            unlock_message: default_unlock_message(),
            hide_taskbar_on_lock: false,
            widget: WidgetConfig::default(),
            unlock_logo_path: None,
            hotkey,
            password_hash: hash_password(password)?,
        };
        self.save(&config)
    }

    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        let mut bytes = Vec::new();
        File::open(&self.path)?.read_to_end(&mut bytes)?;
        let config: AppConfig = serde_json::from_slice(&bytes)
            .map_err(|error| ConfigError::InvalidConfig(error.to_string()))?;
        if config.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion);
        }
        validate_dimming_percentage(config.dimming_percentage)?;
        validate_idle_timeout(config.idle_timeout_minutes)?;
        validate_unlock_message(&config.unlock_message)?;
        validate_widget(&config.widget)?;
        Ok(config)
    }

    pub fn suspend_unstable_features(&self) -> Result<bool, ConfigError> {
        let mut config = self.load()?;
        let changed = config.win_l_enabled;
        if changed {
            config.win_l_enabled = false;
            self.save(&config)?;
        }
        Ok(changed)
    }

    pub fn verify_password(&self, candidate: &str) -> Result<bool, ConfigError> {
        let config = self.load()?;
        verify_password_hash(&config.password_hash, candidate)
    }

    pub fn verify_unlock_password(
        &self,
        candidate: &str,
    ) -> Result<UnlockPasswordResult, ConfigError> {
        let config = self.load()?;
        if config.windows_hello_enabled {
            return Ok(UnlockPasswordResult::DisabledByWindowsHello);
        }
        Ok(if verify_password_hash(&config.password_hash, candidate)? {
            UnlockPasswordResult::Accepted
        } else {
            UnlockPasswordResult::Rejected
        })
    }

    pub fn set_windows_hello_enabled(
        &self,
        current_password: &str,
        enabled: bool,
    ) -> Result<(), ConfigError> {
        let mut config = self.load()?;
        if !verify_password_hash(&config.password_hash, current_password)? {
            return Err(ConfigError::AuthenticationFailed);
        }
        config.windows_hello_enabled = enabled;
        self.save(&config)
    }

    pub fn set_win_l_enabled(
        &self,
        current_password: &str,
        enabled: bool,
    ) -> Result<(), ConfigError> {
        let mut config = self.load()?;
        if !verify_password_hash(&config.password_hash, current_password)? {
            return Err(ConfigError::AuthenticationFailed);
        }
        config.win_l_enabled = enabled;
        self.save(&config)
    }

    pub fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), ConfigError> {
        validate_password_length(new_password)?;
        if !self.verify_password(current_password)? {
            return Err(ConfigError::AuthenticationFailed);
        }
        let mut config = self.load()?;
        config.password_hash = hash_password(new_password)?;
        self.save(&config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        validate_dimming_percentage(config.dimming_percentage)?;
        validate_idle_timeout(config.idle_timeout_minutes)?;
        validate_unlock_message(&config.unlock_message)?;
        validate_widget(&config.widget)?;
        let parent = self.path.parent().ok_or_else(|| {
            ConfigError::InvalidConfig("caminho de configuração sem diretório".into())
        })?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, config)
            .map_err(|error| ConfigError::InvalidConfig(error.to_string()))?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| ConfigError::Io(error.error.to_string()))?;
        sync_directory(parent);
        Ok(())
    }
}

fn validate_unlock_message(message: &str) -> Result<(), ConfigError> {
    if message.chars().count() <= MAX_UNLOCK_MESSAGE_CHARS && !message.chars().any(char::is_control)
    {
        Ok(())
    } else {
        Err(ConfigError::InvalidConfig(
            "a mensagem de desbloqueio deve ter até 80 caracteres e ocupar uma linha".into(),
        ))
    }
}

fn validate_dimming_percentage(percent: u8) -> Result<(), ConfigError> {
    if percent <= 100 {
        Ok(())
    } else {
        Err(ConfigError::InvalidConfig(
            "o escurecimento deve ficar entre 0% e 100%".into(),
        ))
    }
}

pub fn valid_idle_timeout_minutes(minutes: u16) -> bool {
    IDLE_TIMEOUT_OPTIONS_MINUTES.contains(&minutes)
}

fn validate_idle_timeout(minutes: u16) -> Result<(), ConfigError> {
    if valid_idle_timeout_minutes(minutes) {
        Ok(())
    } else {
        Err(ConfigError::InvalidConfig(
            "tempo de inatividade inválido".into(),
        ))
    }
}

fn validate_widget(widget: &WidgetConfig) -> Result<(), ConfigError> {
    if (80..=1200).contains(&widget.width)
        && (40..=800).contains(&widget.height)
        && widget.x_percent <= 100
        && widget.y_percent <= 100
        && widget.opacity_percentage <= 100
    {
        Ok(())
    } else {
        Err(ConfigError::InvalidConfig(
            "tamanho ou posição do widget inválidos".into(),
        ))
    }
}

fn validate_password_length(password: &str) -> Result<(), ConfigError> {
    if password.chars().count() <= 128 {
        Ok(())
    } else {
        Err(ConfigError::InvalidPasswordLength)
    }
}

fn hash_password(password: &str) -> Result<String, ConfigError> {
    let salt = SaltString::generate(&mut password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| ConfigError::InvalidConfig(error.to_string()))
}

fn verify_password_hash(hash: &str, candidate: &str) -> Result<bool, ConfigError> {
    let parsed = PasswordHash::new(hash).map_err(|_| ConfigError::CorruptPasswordHash)?;
    if parsed.algorithm.as_str() != "argon2id" {
        return Err(ConfigError::CorruptPasswordHash);
    }
    match Argon2::default().verify_password(candidate.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(password_hash::Error::Password) => Ok(false),
        Err(_) => Err(ConfigError::CorruptPasswordHash),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}
