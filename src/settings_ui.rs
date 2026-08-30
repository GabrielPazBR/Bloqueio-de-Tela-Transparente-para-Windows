use crate::config::{
    DEFAULT_UNLOCK_MESSAGE, Hotkey, IDLE_TIMEOUT_OPTIONS_MINUTES, MAX_UNLOCK_MESSAGE_CHARS,
    WidgetConfig,
};
use crate::protocol::ClientRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetSizePreset {
    ExtraSmall,
    Small,
    Medium,
    Large,
    ExtraLarge,
}

impl WidgetSizePreset {
    pub const ALL: [Self; 5] = [
        Self::ExtraSmall,
        Self::Small,
        Self::Medium,
        Self::Large,
        Self::ExtraLarge,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ExtraSmall => "Muito pequeno",
            Self::Small => "Pequeno",
            Self::Medium => "Médio",
            Self::Large => "Grande",
            Self::ExtraLarge => "Muito grande",
        }
    }

    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::ExtraSmall => (160, 60),
            Self::Small => (240, 80),
            Self::Medium => (400, 120),
            Self::Large => (640, 200),
            Self::ExtraLarge => (800, 260),
        }
    }

    pub fn from_widget(widget: &WidgetConfig) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.dimensions() == (widget.width, widget.height))
    }

    pub fn apply(self, widget: &mut WidgetConfig) {
        (widget.width, widget.height) = self.dimensions();
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsInputError {
    #[error("as novas senhas não conferem")]
    PasswordConfirmationMismatch,
    #[error("use pelo menos dois modificadores entre Ctrl, Alt e Shift")]
    NotEnoughModifiers,
    #[error("a tecla final deve ser uma letra ou número")]
    InvalidHotkeyKey,
    #[error("o escurecimento deve ficar entre 0% e 100%")]
    InvalidDimmingPercentage,
    #[error("selecione um tempo de inatividade válido")]
    InvalidIdleTimeout,
    #[error("a mensagem deve ter até 80 caracteres e ocupar uma linha")]
    InvalidUnlockMessage,
    #[error("tamanho e posição do widget são inválidos")]
    InvalidWidget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionStatus {
    pub agent_running: bool,
    pub locked: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsModel {
    pub enabled: bool,
    pub windows_hello_enabled: bool,
    pub win_l_enabled: bool,
    pub idle_timeout_minutes: u16,
    pub dimming_percentage: u8,
    pub unlock_message: String,
    pub hide_taskbar_on_lock: bool,
    pub widget: WidgetConfig,
    pub unlock_logo_path: Option<String>,
    pub hotkey: Hotkey,
    pub status: ProtectionStatus,
}

impl SettingsModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        enabled: bool,
        windows_hello_enabled: bool,
        win_l_enabled: bool,
        idle_timeout_minutes: u16,
        dimming_percentage: u8,
        unlock_message: String,
        hide_taskbar_on_lock: bool,
        widget: WidgetConfig,
        unlock_logo_path: Option<String>,
        hotkey: Hotkey,
        status: ProtectionStatus,
    ) -> Self {
        Self {
            enabled,
            windows_hello_enabled,
            win_l_enabled,
            idle_timeout_minutes,
            dimming_percentage,
            unlock_message,
            hide_taskbar_on_lock,
            widget,
            unlock_logo_path,
            hotkey,
            status,
        }
    }

    pub fn set_windows_hello_request(current: &str, enabled: bool) -> ClientRequest {
        ClientRequest::SetWindowsHelloEnabled {
            current: current.into(),
            enabled,
        }
    }

    pub fn set_win_l_request(current: &str, enabled: bool) -> ClientRequest {
        ClientRequest::SetWinLEnabled {
            current: current.into(),
            enabled,
        }
    }

    pub fn status_label(&self) -> &'static str {
        if self.enabled && self.status.agent_running {
            "Proteção ativa"
        } else {
            "Proteção desativada"
        }
    }

    pub fn screen_label(&self) -> &'static str {
        if self.status.locked {
            "Tela bloqueada"
        } else {
            "Tela liberada"
        }
    }

    pub fn change_password_request(
        current: &str,
        new: &str,
        confirmation: &str,
    ) -> Result<ClientRequest, SettingsInputError> {
        if new != confirmation {
            return Err(SettingsInputError::PasswordConfirmationMismatch);
        }
        Ok(ClientRequest::ChangePassword {
            current: current.into(),
            new: new.into(),
        })
    }

    pub fn set_dimming_request(percent: u8) -> Result<ClientRequest, SettingsInputError> {
        if percent > 100 {
            return Err(SettingsInputError::InvalidDimmingPercentage);
        }
        Ok(ClientRequest::SetDimming { percent })
    }

    pub fn set_idle_timeout_request(minutes: u16) -> Result<ClientRequest, SettingsInputError> {
        if !IDLE_TIMEOUT_OPTIONS_MINUTES.contains(&minutes) {
            return Err(SettingsInputError::InvalidIdleTimeout);
        }
        Ok(ClientRequest::SetIdleTimeout { minutes })
    }

    pub fn set_unlock_message_request(message: &str) -> Result<ClientRequest, SettingsInputError> {
        let message = message.trim();
        if message.chars().count() > MAX_UNLOCK_MESSAGE_CHARS
            || message.chars().any(char::is_control)
        {
            return Err(SettingsInputError::InvalidUnlockMessage);
        }
        Ok(ClientRequest::SetUnlockMessage {
            message: if message.is_empty() {
                DEFAULT_UNLOCK_MESSAGE.into()
            } else {
                message.into()
            },
        })
    }

    pub fn set_visual_options_request(
        hide_taskbar_on_lock: bool,
        widget: WidgetConfig,
        unlock_logo_path: Option<String>,
    ) -> Result<ClientRequest, SettingsInputError> {
        if !(80..=1200).contains(&widget.width)
            || !(40..=800).contains(&widget.height)
            || widget.x_percent > 100
            || widget.y_percent > 100
            || widget.opacity_percentage > 100
        {
            return Err(SettingsInputError::InvalidWidget);
        }
        Ok(ClientRequest::SetVisualOptions {
            hide_taskbar_on_lock,
            widget,
            unlock_logo_path: unlock_logo_path.filter(|path| !path.trim().is_empty()),
        })
    }

    pub fn update_hotkey_request(
        current: &str,
        hotkey: Hotkey,
    ) -> Result<ClientRequest, SettingsInputError> {
        let modifier_count = hotkey.control as u8 + hotkey.alt as u8 + hotkey.shift as u8;
        if modifier_count < 2 {
            return Err(SettingsInputError::NotEnoughModifiers);
        }
        if hotkey.key.chars().count() != 1
            || !hotkey
                .key
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(SettingsInputError::InvalidHotkeyKey);
        }
        Ok(ClientRequest::UpdateHotkey {
            current: current.into(),
            hotkey,
        })
    }
}
