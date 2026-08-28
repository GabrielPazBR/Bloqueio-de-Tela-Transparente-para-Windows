use super::{DISPLAY_NAME, SERVICE_NAME, config_path, ipc};
use crate::config::{ConfigStore, Hotkey};
use crate::deployment::ShortcutOptions;
use crate::protocol::{ClientRequest, ServiceResponse};
use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW, SHOW_WINDOW_CMD, SW_HIDE, SW_SHOWNORMAL,
};

pub fn install() -> Result<()> {
    println!("Instalação de {DISPLAY_NAME}");
    println!("Execute este comando em um terminal aberto como administrador.");
    let password = rpassword::prompt_password("Defina a senha: ")?;
    let confirmation = rpassword::prompt_password("Repita a senha: ")?;
    crate::deployment::validate_setup_password(&password, &confirmation)?;
    install_with_password(&password, ShortcutOptions::default())?;
    open_installed_settings()?;
    println!("{DISPLAY_NAME} foi instalado e iniciado.");
    println!("Atalho padrão: Ctrl+Shift+L");
    Ok(())
}

pub fn install_with_password(password: &str, shortcuts: ShortcutOptions) -> Result<()> {
    let target = installed_executable()?;
    let target_directory = target.parent().context("destino de instalação inválido")?;
    let config = config_path()?;
    if target.exists() || config.exists() {
        bail!("já existe uma instalação ou configuração de {DISPLAY_NAME}");
    }

    std::fs::create_dir_all(target_directory)
        .context("não foi possível criar o diretório em Program Files")?;
    let current = std::env::current_exe()?;
    std::fs::copy(&current, &target).context("não foi possível copiar o executável")?;
    let store = ConfigStore::new(config.clone());
    if let Err(error) = store.initialize(password, Hotkey::default()) {
        let _ = std::fs::remove_file(&target);
        return Err(error.into());
    }
    if let Some(directory) = config.parent()
        && let Err(error) = ipc::protect_config_file(directory)
    {
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&config);
        return Err(error.context("não foi possível proteger o diretório de configuração"));
    }
    if let Err(error) = ipc::protect_config_file(&config) {
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&config);
        return Err(error.context("não foi possível proteger o arquivo de configuração"));
    }

    let result = create_and_start_service(&target);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&config);
        return Err(error);
    }
    create_requested_shortcuts(&target, shortcuts)?;
    Ok(())
}

pub fn installation_files() -> (bool, bool) {
    (
        installed_executable().is_ok_and(|path| path.is_file()),
        config_path().is_ok_and(|path| path.is_file()),
    )
}

pub fn installed_version() -> Option<String> {
    let executable = installed_executable().ok()?;
    let output = std::process::Command::new(executable)
        .arg("--app-version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

pub fn request_elevated_setup() -> Result<()> {
    request_elevated("--setup", SW_SHOWNORMAL)
}

pub fn request_elevated_repair() -> Result<()> {
    request_elevated("--repair", SW_HIDE)
}

pub fn request_elevated_update() -> Result<()> {
    request_elevated("--update", SW_HIDE)
}

pub fn request_elevated_uninstall() -> Result<()> {
    request_elevated("--uninstall", SW_HIDE)
}

pub fn request_settings() -> Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg("settings")
        .spawn()
        .context("não foi possível abrir as configurações")?;
    Ok(())
}

pub(super) fn open_installed_settings() -> Result<()> {
    let (executable, argument) = crate::deployment::settings_launch(&installed_executable()?);
    std::process::Command::new(executable)
        .arg(argument)
        .spawn()
        .context("o aplicativo foi instalado, mas não foi possível abrir as configurações")?;
    Ok(())
}

fn request_elevated(command: &str, show_window: SHOW_WINDOW_CMD) -> Result<()> {
    super::settings_window::hide_console();
    let executable = std::env::current_exe()?;
    let operation = wide("runas");
    let executable = wide_os(executable.as_os_str());
    let parameters = wide(command);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            show_window,
        )
    };
    if result as isize <= 32 {
        bail!("a configuração inicial foi cancelada ou não pôde ser elevada");
    }
    Ok(())
}

pub fn repair() -> Result<()> {
    let target = installed_executable()?;
    let config = config_path()?;
    if !config.is_file() {
        bail!("a configuração protegida não foi encontrada; a restauração foi cancelada");
    }

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("não foi possível abrir o Gerenciador de Serviços")?;
    let existing = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )
        .ok();
    if let Some(service) = &existing
        && service.query_status()?.current_state != ServiceState::Stopped
    {
        let _ = service.stop();
        for _ in 0..40 {
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
        if service.query_status()?.current_state != ServiceState::Stopped {
            bail!("não foi possível parar o serviço para restaurar a instalação");
        }
    }

    let current = std::env::current_exe()?;
    let legacy_entry = legacy_start_menu_entry()?;
    let desktop_exists = desktop_entry().is_ok_and(|entry| entry.is_file());
    if current != target && current != legacy_entry {
        if let Some(directory) = target.parent() {
            std::fs::create_dir_all(directory)?;
        }
        std::fs::copy(&current, &target)
            .context("não foi possível restaurar o executável instalado")?;
    }
    create_start_menu_entry(&target)?;
    let _ = std::fs::remove_file(&legacy_entry);
    if desktop_exists {
        create_desktop_entry(&target)?;
    }

    if let Some(service) = existing {
        service
            .start::<&OsStr>(&[])
            .context("os arquivos foram restaurados, mas o serviço não iniciou")?;
    } else {
        create_and_start_service(&target)?;
    }
    Ok(())
}

fn create_and_start_service(target: &std::path::Path) -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE | ServiceManagerAccess::CONNECT,
    )
    .context("acesso negado ao Gerenciador de Serviços; use um terminal como administrador")?;
    let information = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target.to_owned(),
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(&information, ServiceAccess::ALL_ACCESS)
        .context("não foi possível criar o serviço")?;
    if let Err(error) = service.start::<&OsStr>(&[]) {
        let _ = service.delete();
        return Err(error).context("não foi possível iniciar o serviço");
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let _ = super::agent::configure_win_l_override(false);
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("não foi possível abrir o Gerenciador de Serviços")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        )
        .context("serviço não encontrado")?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        let _ = service.stop();
        for _ in 0..20 {
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    service
        .delete()
        .context("não foi possível remover o serviço")?;
    let target = installed_executable()?;
    let config = config_path()?;
    let _ = std::fs::remove_file(start_menu_entry()?);
    let _ = std::fs::remove_file(legacy_start_menu_entry()?);
    if let Ok(entry) = desktop_entry() {
        let _ = std::fs::remove_file(entry);
    }
    let _ = std::fs::remove_file(&config);
    if std::fs::remove_file(&target).is_err() && target.exists() {
        let target_wide = wide_os(target.as_os_str());
        if unsafe {
            MoveFileExW(
                target_wide.as_ptr(),
                std::ptr::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        } == 0
        {
            bail!("o serviço foi removido, mas o executável não pôde ser agendado para exclusão");
        }
    }
    println!(
        "{DISPLAY_NAME} foi removido. O executável pode desaparecer após reiniciar o Windows."
    );
    Ok(())
}

pub fn settings() -> Result<()> {
    super::settings_window::run()
}

pub fn status() -> Result<()> {
    match ipc::send_current_session(&ClientRequest::Status)? {
        ServiceResponse::Status {
            enabled,
            agent_running,
            locked,
            last_error,
        } => {
            println!(
                "Proteção: {}",
                if enabled { "ativada" } else { "desativada" }
            );
            println!(
                "Agente: {}",
                if agent_running {
                    "em execução"
                } else {
                    "parado"
                }
            );
            println!("Tela: {}", if locked { "bloqueada" } else { "liberada" });
            if let Some(error) = last_error {
                println!("Último erro: {error}");
            }
            Ok(())
        }
        response => report_response(response),
    }
}

fn report_response(response: ServiceResponse) -> Result<()> {
    match response {
        ServiceResponse::Ok => {
            println!("Concluído.");
            Ok(())
        }
        ServiceResponse::Error { message } => bail!(message),
        other => bail!("resposta inesperada: {other:?}"),
    }
}

fn installed_executable() -> Result<PathBuf> {
    let root = std::env::var_os("ProgramFiles").context("ProgramFiles não definido")?;
    Ok(PathBuf::from(root)
        .join(DISPLAY_NAME)
        .join("BloqueioTransparente.exe"))
}

fn start_menu_entry() -> Result<PathBuf> {
    let root = std::env::var_os("ProgramData").context("ProgramData nÃ£o definido")?;
    Ok(crate::deployment::start_menu_entry(std::path::Path::new(
        &root,
    )))
}

fn legacy_start_menu_entry() -> Result<PathBuf> {
    let root = std::env::var_os("ProgramData").context("ProgramData não definido")?;
    Ok(crate::deployment::legacy_start_menu_entry(
        std::path::Path::new(&root),
    ))
}

fn desktop_entry() -> Result<PathBuf> {
    let root = std::env::var_os("USERPROFILE").context("USERPROFILE não definido")?;
    Ok(crate::deployment::desktop_entry(std::path::Path::new(
        &root,
    )))
}

fn create_start_menu_entry(target: &std::path::Path) -> Result<()> {
    let entry = start_menu_entry()?;
    create_shell_link(&entry, target).context("não foi possível criar o atalho no menu Iniciar")
}

fn create_desktop_entry(target: &std::path::Path) -> Result<()> {
    let entry = desktop_entry()?;
    create_shell_link(&entry, target).context("não foi possível criar o atalho na área de trabalho")
}

fn create_requested_shortcuts(target: &std::path::Path, options: ShortcutOptions) -> Result<()> {
    if options.start_menu {
        create_start_menu_entry(target)?;
    }
    if options.desktop {
        create_desktop_entry(target)?;
    }
    Ok(())
}

fn create_shell_link(entry: &std::path::Path, target: &std::path::Path) -> Result<()> {
    if let Some(directory) = entry.parent() {
        std::fs::create_dir_all(directory).context("não foi possível acessar a pasta do atalho")?;
    }
    let _ = std::fs::remove_file(entry);
    let target_wide = wide_os(target.as_os_str());
    let entry_wide = wide_os(entry.as_os_str());
    let arguments = wide("settings");
    let description = wide("Abrir as configurações do Bloqueio Transparente");
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .context("não foi possível iniciar a integração com o Windows")?;
    let result = (|| -> windows::core::Result<()> {
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
        unsafe {
            link.SetPath(PCWSTR(target_wide.as_ptr()))?;
            link.SetArguments(PCWSTR(arguments.as_ptr()))?;
            link.SetDescription(PCWSTR(description.as_ptr()))?;
            link.SetIconLocation(PCWSTR(target_wide.as_ptr()), 0)?;
            if let Some(directory) = target.parent() {
                let directory = wide_os(directory.as_os_str());
                link.SetWorkingDirectory(PCWSTR(directory.as_ptr()))?;
            }
            let persist: IPersistFile = link.cast()?;
            persist.Save(PCWSTR(entry_wide.as_ptr()), true)?;
        }
        Ok(())
    })();
    unsafe { CoUninitialize() };
    result.map_err(Into::into)
}

#[derive(Debug, Clone, Copy)]
pub enum ElevatedOperation {
    Repair,
    Update,
    Uninstall,
}

pub fn run_elevated_operation(operation: ElevatedOperation) -> Result<()> {
    super::settings_window::hide_console();
    let result = match operation {
        ElevatedOperation::Repair | ElevatedOperation::Update => repair(),
        ElevatedOperation::Uninstall => uninstall(),
    };
    let action = match operation {
        ElevatedOperation::Repair => "Restauração",
        ElevatedOperation::Update => "Atualização",
        ElevatedOperation::Uninstall => "Desinstalação",
    };
    let (message, flags) = match &result {
        Ok(()) => (
            format!("{action} concluída com sucesso."),
            MB_OK | MB_ICONINFORMATION,
        ),
        Err(error) => (
            format!("Não foi possível concluir a {action}.\n\n{error:#}"),
            MB_OK | MB_ICONERROR,
        ),
    };
    show_result_message(&message, flags);
    result
}

fn show_result_message(
    message: &str,
    flags: windows_sys::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
) {
    let message = wide(message);
    let title = wide(DISPLAY_NAME);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            flags,
        );
    }
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::create_shell_link;

    #[test]
    fn creates_a_real_windows_shell_link() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let entry = directory.path().join("Bloqueio Transparente.lnk");
        create_shell_link(
            &entry,
            &std::env::current_exe().expect("executável de teste"),
        )
        .expect("atalho do Shell");
        assert!(entry.is_file());
        assert!(entry.metadata().expect("metadados").len() > 0);
    }
}
