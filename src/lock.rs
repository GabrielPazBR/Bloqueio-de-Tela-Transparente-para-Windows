use std::time::{Duration, Instant};
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    Disarmed,
    Ready,
    Locked,
    Prompting,
    Verifying,
    FallbackWindowsLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Arm,
    Disarm,
    LockRequested,
    PrintableCharacter(char),
    Backspace,
    CancelPrompt,
    SubmitPassword,
    PasswordAccepted,
    AlternativeAuthenticationAccepted,
    PasswordRejected { retry_after_seconds: u32 },
    RetryDelayElapsed,
    DisplayChanged,
    ConfigurationCorrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    ShowOverlays,
    HideOverlays,
    RebuildOverlays,
    InstallInputHooks,
    RemoveInputHooks,
    ShowPasswordPrompt,
    HidePasswordPrompt,
    ShowPasswordError,
    VerifyPassword(String),
    LockWindows,
}

#[derive(Debug)]
pub struct LockController {
    state: LockState,
    password: String,
    failed_attempts: u32,
    retry_at: Option<Instant>,
}

impl Default for LockController {
    fn default() -> Self {
        Self::new()
    }
}

impl LockController {
    pub fn new() -> Self {
        Self {
            state: LockState::Ready,
            password: String::new(),
            failed_attempts: 0,
            retry_at: None,
        }
    }

    pub fn state(&self) -> LockState {
        self.state
    }

    pub fn password_buffer(&self) -> &str {
        &self.password
    }

    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    pub fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::Arm if self.state == LockState::Disarmed => {
                self.state = LockState::Ready;
                vec![]
            }
            Event::Disarm if self.state == LockState::Ready => {
                self.state = LockState::Disarmed;
                vec![]
            }
            Event::LockRequested if self.state == LockState::Ready => {
                self.state = LockState::Locked;
                vec![Action::ShowOverlays, Action::InstallInputHooks]
            }
            Event::PrintableCharacter(ch)
                if matches!(self.state, LockState::Locked | LockState::Prompting)
                    && self.retry_at.is_none()
                    && self.password.chars().count() < 128 =>
            {
                let show = self.state == LockState::Locked;
                self.state = LockState::Prompting;
                self.password.push(ch);
                if show {
                    vec![Action::ShowPasswordPrompt]
                } else {
                    vec![]
                }
            }
            Event::Backspace if self.state == LockState::Prompting => {
                self.password.pop();
                vec![]
            }
            Event::CancelPrompt if self.state == LockState::Prompting => {
                self.clear_password();
                self.state = LockState::Locked;
                vec![Action::HidePasswordPrompt]
            }
            Event::SubmitPassword
                if matches!(self.state, LockState::Locked | LockState::Prompting)
                    && self.retry_at.is_none() =>
            {
                let show = self.state == LockState::Locked;
                self.state = LockState::Verifying;
                let candidate = self.password.clone();
                self.clear_password();
                if show {
                    vec![
                        Action::ShowPasswordPrompt,
                        Action::VerifyPassword(candidate),
                    ]
                } else {
                    vec![Action::VerifyPassword(candidate)]
                }
            }
            Event::PasswordAccepted if self.state == LockState::Verifying => {
                self.failed_attempts = 0;
                self.retry_at = None;
                self.state = LockState::Ready;
                vec![Action::RemoveInputHooks, Action::HideOverlays]
            }
            Event::AlternativeAuthenticationAccepted if Self::is_locked_state(self.state) => {
                self.clear_password();
                self.failed_attempts = 0;
                self.retry_at = None;
                self.state = LockState::Ready;
                vec![Action::RemoveInputHooks, Action::HideOverlays]
            }
            Event::PasswordRejected {
                retry_after_seconds,
            } if self.state == LockState::Verifying => {
                self.failed_attempts = self.failed_attempts.saturating_add(1);
                self.state = LockState::Prompting;
                if retry_after_seconds > 0 {
                    self.retry_at = Some(now + Duration::from_secs(retry_after_seconds.into()));
                }
                vec![Action::ShowPasswordError]
            }
            Event::RetryDelayElapsed if self.retry_at.is_some_and(|deadline| now >= deadline) => {
                self.retry_at = None;
                vec![]
            }
            Event::DisplayChanged if Self::is_locked_state(self.state) => {
                vec![Action::RebuildOverlays]
            }
            Event::ConfigurationCorrupt if Self::is_locked_state(self.state) => {
                self.clear_password();
                self.state = LockState::FallbackWindowsLock;
                vec![Action::LockWindows]
            }
            _ => vec![],
        }
    }

    fn is_locked_state(state: LockState) -> bool {
        matches!(
            state,
            LockState::Locked | LockState::Prompting | LockState::Verifying
        )
    }

    fn clear_password(&mut self) {
        self.password.zeroize();
        self.password.clear();
    }
}

impl Drop for LockController {
    fn drop(&mut self) {
        self.clear_password();
    }
}
