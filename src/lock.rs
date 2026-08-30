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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnlockMethod {
    #[default]
    Password,
    WindowsHello,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Arm,
    Disarm,
    LockRequested,
    PrintableCharacter(char),
    Backspace,
    CancelPrompt,
    PromptInactivityElapsed,
    SubmitPassword,
    UserInteraction,
    PasswordAccepted,
    AlternativeAuthenticationAccepted,
    AlternativeAuthenticationRejected,
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
    VerifyWindowsHello,
    CancelWindowsHello,
    LockWindows,
}

#[derive(Debug)]
pub struct LockController {
    state: LockState,
    password: String,
    failed_attempts: u32,
    retry_at: Option<Instant>,
    unlock_method: UnlockMethod,
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
            unlock_method: UnlockMethod::Password,
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

    pub fn set_unlock_method(&mut self, method: UnlockMethod) {
        if matches!(self.state, LockState::Disarmed | LockState::Ready) {
            self.unlock_method = method;
        }
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
            Event::UserInteraction
                if self.state == LockState::Locked
                    && self.unlock_method == UnlockMethod::WindowsHello =>
            {
                self.start_windows_hello_verification()
            }
            Event::UserInteraction if self.state == LockState::Locked => {
                self.state = LockState::Prompting;
                vec![Action::ShowPasswordPrompt]
            }
            Event::PrintableCharacter(ch)
                if matches!(self.state, LockState::Locked | LockState::Prompting)
                    && self.retry_at.is_none()
                    && self.password.chars().count() < 128 =>
            {
                if self.unlock_method == UnlockMethod::WindowsHello {
                    return self.start_windows_hello_verification();
                }
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
                if self.unlock_method == UnlockMethod::WindowsHello {
                    return self.start_windows_hello_verification();
                }
                self.password.pop();
                vec![]
            }
            Event::CancelPrompt if self.state == LockState::Prompting => {
                if self.unlock_method == UnlockMethod::WindowsHello {
                    return self.start_windows_hello_verification();
                }
                self.clear_password();
                self.state = LockState::Locked;
                vec![Action::HidePasswordPrompt]
            }
            Event::PromptInactivityElapsed if self.state == LockState::Prompting => {
                self.clear_password();
                self.state = LockState::Locked;
                vec![Action::HidePasswordPrompt]
            }
            Event::PromptInactivityElapsed
                if self.state == LockState::Verifying
                    && self.unlock_method == UnlockMethod::WindowsHello =>
            {
                self.clear_password();
                self.state = LockState::Locked;
                vec![
                    Action::CancelWindowsHello,
                    Action::HidePasswordPrompt,
                    Action::InstallInputHooks,
                ]
            }
            Event::SubmitPassword
                if matches!(self.state, LockState::Locked | LockState::Prompting)
                    && self.retry_at.is_none() =>
            {
                if self.unlock_method == UnlockMethod::WindowsHello {
                    return self.start_windows_hello_verification();
                }
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
            Event::AlternativeAuthenticationRejected if self.state == LockState::Verifying => {
                self.state = LockState::Locked;
                vec![Action::InstallInputHooks, Action::ShowPasswordError]
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

    fn start_windows_hello_verification(&mut self) -> Vec<Action> {
        self.clear_password();
        self.state = LockState::Verifying;
        vec![Action::ShowPasswordPrompt, Action::VerifyWindowsHello]
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
