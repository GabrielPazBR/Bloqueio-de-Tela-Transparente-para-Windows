use crate::config::{Hotkey, WidgetConfig, default_unlock_message};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

pub const MAX_FRAME_BYTES: usize = 16 * 1024;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([oculto])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ClientRequest {
    Lock,
    Status,
    Settings,
    Heartbeat {
        locked: bool,
    },
    VerifyPassword {
        candidate: SecretString,
    },
    ChangePassword {
        current: SecretString,
        new: SecretString,
    },
    SetEnabled {
        current: SecretString,
        enabled: bool,
    },
    SetDimming {
        percent: u8,
    },
    SetUnlockMessage {
        message: String,
    },
    SetVisualOptions {
        hide_taskbar_on_lock: bool,
        widget: WidgetConfig,
        unlock_logo_path: Option<String>,
    },
    UpdateHotkey {
        current: SecretString,
        hotkey: Hotkey,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ServiceResponse {
    Ok,
    PasswordAccepted,
    PasswordRejected {
        retry_after_seconds: u32,
    },
    Status {
        enabled: bool,
        agent_running: bool,
        locked: bool,
        last_error: Option<String>,
    },
    Settings {
        enabled: bool,
        #[serde(default)]
        dimming_percentage: u8,
        #[serde(default = "default_unlock_message")]
        unlock_message: String,
        #[serde(default)]
        hide_taskbar_on_lock: bool,
        #[serde(default)]
        widget: WidgetConfig,
        #[serde(default)]
        unlock_logo_path: Option<String>,
        hotkey: Hotkey,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("quadro maior que o limite permitido")]
    FrameTooLarge,
    #[error("quadro incompleto")]
    IncompleteFrame,
    #[error("mensagem inválida: {0}")]
    InvalidMessage(String),
}

pub struct CommandCodec;

impl CommandCodec {
    pub fn encode_request(request: &ClientRequest) -> Result<Vec<u8>, ProtocolError> {
        encode(request)
    }

    pub fn decode_request(frame: &[u8]) -> Result<ClientRequest, ProtocolError> {
        decode(frame)
    }

    pub fn encode_response(response: &ServiceResponse) -> Result<Vec<u8>, ProtocolError> {
        encode(response)
    }

    pub fn decode_response(frame: &[u8]) -> Result<ServiceResponse, ProtocolError> {
        decode(frame)
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| ProtocolError::InvalidMessage(error.to_string()))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, ProtocolError> {
    if frame.len() < 4 {
        return Err(ProtocolError::IncompleteFrame);
    }
    let length = u32::from_le_bytes(frame[..4].try_into().expect("four bytes")) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    if frame.len() != length + 4 {
        return Err(ProtocolError::IncompleteFrame);
    }
    serde_json::from_slice(&frame[4..])
        .map_err(|error| ProtocolError::InvalidMessage(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeNames {
    pub control: String,
    pub agent: String,
}

impl PipeNames {
    pub fn for_session(session_id: u32) -> Self {
        Self {
            control: format!(r"\\.\pipe\BloqueioTransparente.Control.{session_id}"),
            agent: format!(r"\\.\pipe\BloqueioTransparente.Agent.{session_id}"),
        }
    }
}
