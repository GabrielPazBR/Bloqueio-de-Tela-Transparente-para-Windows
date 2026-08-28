mod agent;
mod install;
mod ipc;
mod maintenance_window;
mod service;
mod settings_window;
mod setup_window;

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
        Some("--setup") => setup_window::run(),
        Some("--repair") => install::run_elevated_operation(install::ElevatedOperation::Repair),
        Some("--update") => install::run_elevated_operation(install::ElevatedOperation::Update),
        Some("--uninstall") => {
            install::run_elevated_operation(install::ElevatedOperation::Uninstall)
        }
        Some("--app-version") => {
            println!(env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("install") => install::install(),
        Some("uninstall") => install::uninstall(),
        Some("lock") => ipc::send_current_session(&crate::protocol::ClientRequest::Lock)
            .context("não foi possível solicitar o bloqueio")
            .map(|_| ()),
        Some("settings") => install::settings(),
        Some("status") => install::status(),
        Some(command) => bail!("comando desconhecido: {command}"),
        None => {
            let (executable_exists, config_exists) = install::installation_files();
            match crate::deployment::first_run_action(executable_exists, config_exists) {
                crate::deployment::FirstRunAction::RequestElevatedSetup => {
                    install::request_elevated_setup()
                }
                crate::deployment::FirstRunAction::OpenMaintenance => maintenance_window::run(),
            }
        }
    }
}

pub fn config_path() -> Result<std::path::PathBuf> {
    let root = env::var_os("ProgramData").context("ProgramData não definido")?;
    Ok(std::path::PathBuf::from(root)
        .join(DISPLAY_NAME)
        .join("config.json"))
}
