mod background;
#[cfg(any(windows, test))]
mod taskbar_recovery;

pub mod config;
pub mod deployment;
pub mod lock;
pub mod package;
pub mod protocol;
pub mod rate_limit;
pub mod settings_ui;
pub mod watchdog;
pub mod windows_policy;

#[cfg(windows)]
pub mod windows_app;
