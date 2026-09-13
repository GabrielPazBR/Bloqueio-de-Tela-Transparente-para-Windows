mod agent;
mod install;
mod ipc;
mod maintenance_window;
mod package_registration;
mod service;
mod settings_window;
mod setup_window;
mod windows_hello;

use anyhow::{Context, Result, bail};
use std::env;

pub const SERVICE_NAME: &str = "BloqueioTransparente";
pub const DISPLAY_NAME: &str = "Bloqueio Transparente";

pub fn run() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("--agent") => agent::run(arguments.any(|argument| argument == "--locked")),
        Some("--service") => service::dispatch(),
        Some("--fallback-lock") => agent::lock_windows(),
        Some("--setup") => install::launch_cached_installer(""),
        Some("--repair") => install::launch_cached_installer("--repair"),
        Some("--update") => install::launch_cached_installer(""),
        Some("--uninstall") => install::launch_cached_installer("--uninstall"),
        Some("--app-version") => {
            println!(env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--app-architecture") => {
            println!("{}", crate::deployment::binary_architecture());
            Ok(())
        }
        Some("install") => install::launch_cached_installer(""),
        Some("uninstall") => install::launch_cached_installer("--uninstall"),
        Some("lock") => ipc::send_current_session(&crate::protocol::ClientRequest::Lock)
            .context("não foi possível solicitar o bloqueio")
            .map(|_| ()),
        Some("settings") => install::settings(),
        Some("status") => install::status(),
        Some(command) => bail!("comando desconhecido: {command}"),
        None => {
            // The protected configuration is deliberately unreadable by a
            // standard user. Detect installation through Program Files.
            if !install::installation_files().0 {
                bail!("Execute o instalador do Bloqueio Transparente para instalar o aplicativo.");
            }
            install::settings()
        }
    }
}

pub fn run_installer(payload: &'static [u8]) -> Result<()> {
    install::set_payload(payload)?;
    match env::args().nth(1).as_deref() {
        Some("--repair-quiet" | "--update-quiet") => install::repair(),
        Some("--repair") => install::run_elevated_operation(install::ElevatedOperation::Repair),
        Some("--update") => install::run_elevated_operation(install::ElevatedOperation::Update),
        Some("--uninstall") => {
            install::run_elevated_operation(install::ElevatedOperation::Uninstall)
        }
        Some("--setup") => setup_window::run(),
        Some("install") => install::install(),
        Some(command) => bail!("comando desconhecido: {command}"),
        None => {
            let (executable, config) = install::installation_files();
            match crate::deployment::first_run_action(executable, config) {
                crate::deployment::FirstRunAction::RequestElevatedSetup => setup_window::run(),
                crate::deployment::FirstRunAction::OpenMaintenance => maintenance_window::run(),
            }
        }
    }
}

pub fn show_installer_error(message: &str) {
    install::show_error(message);
}

pub fn config_path() -> Result<std::path::PathBuf> {
    Ok(
        known_folder(&windows::Win32::UI::Shell::FOLDERID_ProgramData)?
            .join(DISPLAY_NAME)
            .join("config.json"),
    )
}

pub(super) fn known_folder(id: &windows::core::GUID) -> Result<std::path::PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    let value = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }?;
    let path = unsafe { value.to_string() };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    Ok(std::path::PathBuf::from(path?))
}
