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

static PAYLOAD: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

pub(super) fn set_payload(payload: &'static [u8]) -> Result<()> {
    if !payload.starts_with(b"MZ") {
        bail!("o pacote não contém um aplicativo Windows válido");
    }
    PAYLOAD
        .set(payload)
        .map_err(|_| anyhow::anyhow!("pacote já inicializado"))
}

fn payload() -> Result<&'static [u8]> {
    PAYLOAD
        .get()
        .copied()
        .context("execute o instalador dedicado para instalar ou reparar o aplicativo")
}

pub fn install_with_password(password: &str, shortcuts: ShortcutOptions) -> Result<()> {
    apply_package(Some(password), shortcuts)
}

fn apply_package(password: Option<&str>, shortcuts: ShortcutOptions) -> Result<()> {
    let _operation = OperationGuard::acquire()?;
    let bytes = payload()?;
    let target = installed_executable()?;
    let config = config_path()?;
    let fresh = password.is_some();
    if fresh && (target.exists() || config.exists()) {
        bail!("já existe uma instalação ou configuração; use Atualizar ou Reparar");
    }
    if !fresh {
        ConfigStore::new(config.clone())
            .load()
            .context("a configuração existente não pôde ser lida; os dados foram preservados")?;
        if let Some(version) = installed_version()
            && crate::package::is_downgrade(&version, env!("CARGO_PKG_VERSION"))
        {
            bail!(
                "há uma versão mais recente instalada ({version}); use um instalador dessa versão ou posterior"
            );
        }
    }
    let current = std::env::current_exe()?;
    let cached = cached_installer()?;
    // Read the package before stopping a working installation.
    let installer = std::fs::read(&current).context("não foi possível ler o instalador")?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let existing = open_optional_service(&manager)?;
    let original_config = existing
        .as_ref()
        .map(|service| service.query_config())
        .transpose()?;
    let was_running = existing
        .as_ref()
        .map(|service| service.query_status())
        .transpose()?
        .is_some_and(|status| status.current_state != ServiceState::Stopped);
    if fresh && existing.is_some() {
        bail!("o serviço já existe; use Reparar instalação");
    }
    if let Some(service) = &existing {
        stop_service(service)?;
    }

    let mut files = crate::package::FileTransaction::default();
    let mut service = existing;
    let mut created_service = false;
    let mut configuration_created = false;
    let result = (|| -> Result<super::package_registration::Registration> {
        files
            .replace(&target, bytes)
            .context("não foi possível instalar o aplicativo")?;
        if current != cached {
            files
                .replace(&cached, &installer)
                .context("não foi possível guardar o instalador para reparação")?;
        }
        if let Some(password) = password {
            let directory = config
                .parent()
                .context("diretório de configuração inválido")?;
            std::fs::create_dir_all(directory)?;
            ipc::protect_config_file(directory)?;
            ConfigStore::new(config.clone()).initialize(password, Hotkey::default())?;
            configuration_created = true;
            ipc::protect_config_file(&config)?;
        }
        if shortcuts.start_menu {
            files.track(&start_menu_entry()?)?;
        }
        if shortcuts.desktop {
            files.track(&desktop_entry()?)?;
        }
        create_requested_shortcuts(&target, shortcuts)?;
        let legacy = legacy_start_menu_entry()?;
        if legacy.is_file() {
            files.track(&legacy)?;
            std::fs::remove_file(&legacy)?;
        }
        if service.is_none() {
            service = Some(
                manager.create_service(&service_information(&target), ServiceAccess::ALL_ACCESS)?,
            );
            created_service = true;
        }
        let service = service.as_ref().context("serviço indisponível")?;
        set_service_command(
            service,
            &service_command(&target),
            ServiceStartType::AutoStart,
        )?;
        start_service(service).context(
            "o Windows não iniciou o aplicativo; confira os eventos de Controle de Aplicativo",
        )?;
        register_installation(&target, &cached)
    })();
    match result {
        Ok(registration) => {
            files.commit();
            registration.commit();
            Ok(())
        }
        Err(error) => {
            let mut rollback_errors = Vec::new();
            // A running process must release the replacement before rollback.
            if let Some(service) = &service {
                if let Err(failure) = stop_service(service) {
                    rollback_errors.push(failure.to_string());
                }
                if created_service {
                    if let Err(failure) = service.delete() {
                        rollback_errors.push(failure.to_string());
                    }
                } else if let Some(previous) = &original_config
                    && let Err(failure) = set_service_command(
                        service,
                        previous.executable_path.as_os_str(),
                        previous.start_type,
                    )
                {
                    rollback_errors.push(failure.to_string());
                }
            }
            if let Err(failure) = files.rollback() {
                rollback_errors.push(failure.to_string());
            }
            if configuration_created && let Err(failure) = std::fs::remove_file(&config) {
                rollback_errors.push(failure.to_string());
            }
            if was_running
                && let Some(service) = &service
                && let Err(failure) = start_service(service)
            {
                rollback_errors.push(format!("reinício da versão anterior: {failure}"));
            }
            if !rollback_errors.is_empty() {
                bail!(
                    "{error:#}. Falhas ao restaurar a instalação anterior: {}",
                    rollback_errors.join("; ")
                );
            }
            Err(error.context("a operação falhou; os arquivos anteriores foram restaurados"))
        }
    }
}

pub fn installation_files() -> (bool, bool) {
    (
        installed_executable().is_ok_and(|path| path.is_file()),
        config_path().is_ok_and(|path| path.is_file()),
    )
}

pub fn installed_version() -> Option<String> {
    file_version(&installed_executable().ok()?)
}

fn file_version(path: &std::path::Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    let path = wide_os(path.as_os_str());
    unsafe {
        let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0_u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut info = std::ptr::null_mut();
        let mut length = 0;
        if VerQueryValueW(
            data.as_ptr().cast(),
            wide("\\").as_ptr(),
            &mut info,
            &mut length,
        ) == 0
            || info.is_null()
            || length < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
        {
            return None;
        }
        let info = std::ptr::read_unaligned(info.cast::<VS_FIXEDFILEINFO>());
        if info.dwSignature != 0xfeef04bd {
            return None;
        }
        Some(format!(
            "{}.{}.{}",
            info.dwFileVersionMS >> 16,
            info.dwFileVersionMS & 0xffff,
            info.dwFileVersionLS >> 16
        ))
    }
}

pub fn launch_cached_installer(command: &str) -> Result<()> {
    let installer = cached_installer()?;
    if !installer.is_file() {
        bail!("o instalador de manutenção não foi encontrado; baixe e abra o instalador completo");
    }
    launch_elevated(&installer, command, SW_SHOWNORMAL)
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
    open_installed_settings()
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
    launch_elevated(&std::env::current_exe()?, command, show_window)
}

fn launch_elevated(
    executable: &std::path::Path,
    command: &str,
    show_window: SHOW_WINDOW_CMD,
) -> Result<()> {
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
        bail!("a operação foi cancelada ou não pôde ser elevada");
    }
    Ok(())
}

pub fn repair() -> Result<()> {
    apply_package(
        None,
        ShortcutOptions {
            start_menu: true,
            desktop: desktop_entry().is_ok_and(|entry| entry.is_file()),
        },
    )
}

fn service_information(target: &std::path::Path) -> ServiceInfo {
    ServiceInfo {
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
    }
}

fn open_optional_service(
    manager: &ServiceManager,
) -> Result<Option<windows_service::service::Service>> {
    match manager.open_service(SERVICE_NAME, ServiceAccess::ALL_ACCESS) {
        Ok(service) => Ok(Some(service)),
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn stop_service(service: &windows_service::service::Service) -> Result<()> {
    let state = service.query_status()?.current_state;
    if state == ServiceState::Stopped {
        return Ok(());
    }
    if state != ServiceState::StopPending {
        service.stop()?;
    }
    for _ in 0..80 {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("o serviço não parou; os arquivos em uso não serão substituídos")
}

fn start_service(service: &windows_service::service::Service) -> Result<()> {
    if service.query_status()?.current_state == ServiceState::Running {
        return Ok(());
    }
    service.start::<&OsStr>(&[])?;
    for _ in 0..80 {
        let status = service.query_status()?;
        if status.current_state == ServiceState::Running {
            return Ok(());
        }
        if status.current_state == ServiceState::Stopped {
            bail!(
                "o serviço encerrou durante a inicialização: {:?}",
                status.exit_code
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("o serviço não confirmou a inicialização")
}

fn service_command(target: &std::path::Path) -> OsString {
    let mut command = OsString::from("\"");
    command.push(target.as_os_str());
    command.push("\" --service");
    command
}

fn set_service_command(
    service: &windows_service::service::Service,
    command: &OsStr,
    start_type: ServiceStartType,
) -> Result<()> {
    use windows_sys::Win32::System::Services::{ChangeServiceConfigW, SERVICE_NO_CHANGE};
    let command = wide_os(command);
    if unsafe {
        ChangeServiceConfigW(
            service.raw_handle(),
            SERVICE_NO_CHANGE,
            start_type.to_raw(),
            SERVICE_NO_CHANGE,
            command.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("não foi possível reparar o serviço");
    }
    Ok(())
}

fn cached_installer() -> Result<PathBuf> {
    Ok(installed_executable()?
        .parent()
        .context("destino inválido")?
        .join("Installer")
        .join(format!(
            "BloqueioTransparente-Setup-{}.exe",
            env!("CARGO_PKG_VERSION")
        )))
}

fn register_installation(
    target: &std::path::Path,
    cached: &std::path::Path,
) -> Result<super::package_registration::Registration> {
    let mut registration = super::package_registration::Registration::open()?;
    let result = (|| -> Result<()> {
        registration.text("DisplayName", DISPLAY_NAME)?;
        registration.text("DisplayVersion", env!("CARGO_PKG_VERSION"))?;
        registration.text("Publisher", "Gabriel Paz")?;
        registration.text(
            "InstallLocation",
            &target
                .parent()
                .context("destino inválido")?
                .to_string_lossy(),
        )?;
        registration.text("DisplayIcon", &format!("\"{}\",0", target.display()))?;
        registration.text("ModifyPath", &format!("\"{}\"", cached.display()))?;
        registration.text(
            "UninstallString",
            &format!("\"{}\" --uninstall", cached.display()),
        )?;
        registration.number("NoModify", 0)?;
        registration.number("NoRepair", 0)?;
        registration.number("WindowsInstaller", 0)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(rollback) = registration.rollback() {
            bail!("{error:#}; {rollback:#}");
        }
        return Err(error);
    }
    Ok(registration)
}

struct OperationGuard(windows_sys::Win32::Foundation::HANDLE);
impl OperationGuard {
    fn acquire() -> Result<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let handle = unsafe {
            CreateMutexW(
                std::ptr::null(),
                1,
                wide(r"Global\BloqueioTransparente.Install").as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            bail!("outra instalação ou reparação já está em andamento");
        }
        Ok(Self(handle))
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub fn uninstall() -> Result<()> {
    let _operation = OperationGuard::acquire()?;
    let _ = super::service::restore_win_l_for_current_user();
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    if let Some(service) = open_optional_service(&manager)? {
        stop_service(&service)?;
        service.delete()?;
    }
    for entry in [start_menu_entry()?, legacy_start_menu_entry()?] {
        remove_or_schedule(&entry)?;
    }
    if let Ok(entry) = desktop_entry() {
        remove_or_schedule(&entry)?;
    }
    let target = installed_executable()?;
    remove_or_schedule(&target)?;
    let cache = cached_installer()?
        .parent()
        .context("diretório inválido")?
        .to_owned();
    if cache.is_dir() {
        for entry in std::fs::read_dir(&cache)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_file()
                && name.starts_with("BloqueioTransparente-Setup-")
                && name.ends_with(".exe")
            {
                remove_or_schedule(&entry.path())?;
            }
        }
    }
    super::package_registration::remove()?;
    // Keep the protected configuration for reinstalling or repairing later.
    Ok(())
}

fn remove_or_schedule(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if std::fs::remove_file(path).is_ok() {
        return Ok(());
    }
    let retired = crate::package::retire_file(path)?;
    let retired_wide = wide_os(retired.as_os_str());
    if unsafe {
        MoveFileExW(
            retired_wide.as_ptr(),
            std::ptr::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        if !path.exists() {
            let _ = std::fs::rename(&retired, path);
        }
        return Err(error).with_context(|| {
            format!(
                "não foi possível agendar a remoção; confira {} e {}",
                path.display(),
                retired.display()
            )
        });
    }
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
    use windows::Win32::UI::Shell::{
        FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86,
    };
    let root = super::known_folder(&FOLDERID_ProgramFiles)?;
    let mut roots = vec![root.clone()];
    for id in [FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86] {
        if let Ok(path) = super::known_folder(&id)
            && !roots.contains(&path)
        {
            roots.push(path);
        }
    }
    let existing: Vec<_> = roots
        .into_iter()
        .map(|root| root.join(DISPLAY_NAME))
        .filter(|directory| {
            directory.join("BloqueioTransparente.exe").exists()
                || directory.join("Installer").is_dir()
        })
        .collect();
    if existing.len() > 1 {
        bail!(
            "há instalações em mais de uma pasta Program Files; remova a instalação duplicada antes de continuar"
        );
    }
    Ok(existing
        .into_iter()
        .next()
        .unwrap_or_else(|| root.join(DISPLAY_NAME))
        .join("BloqueioTransparente.exe"))
}

fn start_menu_entry() -> Result<PathBuf> {
    Ok(
        super::known_folder(&windows::Win32::UI::Shell::FOLDERID_CommonPrograms)?
            .join("Bloqueio Transparente.lnk"),
    )
}

fn legacy_start_menu_entry() -> Result<PathBuf> {
    Ok(
        super::known_folder(&windows::Win32::UI::Shell::FOLDERID_CommonPrograms)?
            .join("Bloqueio Transparente.exe"),
    )
}

fn desktop_entry() -> Result<PathBuf> {
    Ok(
        super::known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)?
            .join("Bloqueio Transparente.lnk"),
    )
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

pub(super) fn show_error(message: &str) {
    show_result_message(message, MB_OK | MB_ICONERROR);
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
