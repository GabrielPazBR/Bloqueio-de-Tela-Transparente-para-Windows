use super::{SERVICE_NAME, config_path, ipc};
use crate::config::{ConfigStore, UnlockPasswordResult};
use crate::protocol::{ClientRequest, PipeNames, ServiceResponse};
use crate::rate_limit::{RateLimitDecision, RateLimiter};
use crate::watchdog::{Watchdog, WatchdogAction};
use crate::windows_policy::trusted_agent_process;
use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType, SessionChangeReason,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::EventLog::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken,
};
use windows_sys::Win32::System::Threading::*;

define_windows_service!(ffi_service_main, service_main);

enum ServiceEvent {
    Stop,
    SessionChanged {
        reason: SessionChangeReason,
        session_id: u32,
    },
    PowerChanged,
}

struct ServiceRuntime {
    config: ConfigStore,
    watchdog: Watchdog,
    enabled: bool,
    agent_running: bool,
    locked: bool,
    recovery_paused: bool,
    shutdown_requested: bool,
    last_error: Option<String>,
    rate_limiter: RateLimiter,
    agent_process: HANDLE,
}

unsafe impl Send for ServiceRuntime {}

impl Drop for ServiceRuntime {
    fn drop(&mut self) {
        self.terminate_agent();
    }
}

impl ServiceRuntime {
    fn agent_has_exited(&self) -> bool {
        !self.agent_process.is_null()
            && unsafe { WaitForSingleObject(self.agent_process, 0) } == WAIT_OBJECT_0
    }

    fn terminate_agent(&mut self) {
        if !self.agent_process.is_null() {
            unsafe {
                TerminateProcess(self.agent_process, 1);
                CloseHandle(self.agent_process);
            }
            self.agent_process = null_mut();
        }
        self.agent_running = false;
    }

    fn suspend_recovery_after_windows_fallback(&mut self, message: &str) {
        self.terminate_agent();
        self.recovery_paused = true;
        self.locked = false;
        self.last_error = Some(message.to_owned());
        self.watchdog.heartbeat(Instant::now(), false);
    }

    fn handle_request(&mut self, request: ClientRequest, client_process: u32) -> ServiceResponse {
        match request {
            ClientRequest::Shutdown => {
                if !self.is_agent_process(client_process) {
                    return ServiceResponse::Error {
                        message: "cliente do agente não autorizado".into(),
                    };
                }
                self.shutdown_requested = true;
                ServiceResponse::Ok
            }
            ClientRequest::Heartbeat { locked } => {
                if !self.is_agent_process(client_process) {
                    return ServiceResponse::Error {
                        message: "cliente do agente não autorizado".into(),
                    };
                }
                self.agent_running = true;
                self.locked = locked;
                self.watchdog.heartbeat(Instant::now(), locked);
                ServiceResponse::Ok
            }
            ClientRequest::ApplyWinLPolicy { enabled } => {
                if !self.is_agent_process(client_process) {
                    return ServiceResponse::Error {
                        message: "cliente do agente não autorizado".into(),
                    };
                }
                let mut session = 0;
                if unsafe { ProcessIdToSessionId(client_process, &mut session) } == 0 {
                    return ServiceResponse::Error {
                        message: "não foi possível identificar a sessão do agente".into(),
                    };
                }
                match configure_win_l_for_session(session, enabled) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::Status => ServiceResponse::Status {
                enabled: self.enabled,
                agent_running: self.agent_running,
                locked: self.locked,
                last_error: self.last_error.clone(),
            },
            ClientRequest::VerifyPassword { candidate } => {
                if !self.is_agent_process(client_process) {
                    return ServiceResponse::Error {
                        message: "cliente do agente não autorizado".into(),
                    };
                }
                self.verify(candidate.expose())
            }
            ClientRequest::ChangePassword { current, new } => {
                if let Err(response) = self.require_authentication(current.expose()) {
                    return response;
                }
                match self.config.change_password(current.expose(), new.expose()) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetEnabled { current, enabled } => {
                if let Err(response) = self.require_authentication(current.expose()) {
                    return response;
                }
                match self.config.load().and_then(|mut config| {
                    config.enabled = enabled;
                    self.config.save(&config)
                }) {
                    Ok(()) => {
                        self.enabled = enabled;
                        self.recovery_paused = false;
                        ServiceResponse::Ok
                    }
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetWindowsHelloEnabled { current, enabled } => {
                if let Err(response) = self.require_authentication(current.expose()) {
                    return response;
                }
                match self
                    .config
                    .set_windows_hello_enabled(current.expose(), enabled)
                {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetWinLEnabled { current, enabled } => {
                if let Err(response) = self.require_authentication(current.expose()) {
                    return response;
                }
                match self.config.set_win_l_enabled(current.expose(), enabled) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetDimming { percent } => {
                if percent > 100 {
                    return ServiceResponse::Error {
                        message: "o escurecimento deve ficar entre 0% e 100%".into(),
                    };
                }
                match self.config.load().and_then(|mut config| {
                    config.dimming_percentage = percent;
                    self.config.save(&config)
                }) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetIdleTimeout { minutes } => {
                if !crate::config::valid_idle_timeout_minutes(minutes) {
                    return ServiceResponse::Error {
                        message: "tempo de inatividade inválido".into(),
                    };
                }
                match self.config.load().and_then(|mut config| {
                    config.idle_timeout_minutes = minutes;
                    self.config.save(&config)
                }) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetUnlockMessage { message } => {
                match self.config.load().and_then(|mut config| {
                    config.unlock_message = message;
                    self.config.save(&config)
                }) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::SetVisualOptions {
                hide_taskbar_on_lock,
                widget,
                unlock_logo_path,
            } => match self.config.load().and_then(|mut config| {
                config.hide_taskbar_on_lock = hide_taskbar_on_lock;
                config.widget = widget;
                config.unlock_logo_path = unlock_logo_path;
                self.config.save(&config)
            }) {
                Ok(()) => ServiceResponse::Ok,
                Err(error) => ServiceResponse::Error {
                    message: error.to_string(),
                },
            },
            ClientRequest::UpdateHotkey { current, hotkey } => {
                if let Err(response) = self.require_authentication(current.expose()) {
                    return response;
                }
                match self.config.load().and_then(|mut config| {
                    config.hotkey = hotkey;
                    self.config.save(&config)
                }) {
                    Ok(()) => ServiceResponse::Ok,
                    Err(error) => ServiceResponse::Error {
                        message: error.to_string(),
                    },
                }
            }
            ClientRequest::Settings => match self.config.load() {
                Ok(config) => ServiceResponse::Settings {
                    enabled: config.enabled,
                    windows_hello_enabled: config.windows_hello_enabled,
                    win_l_enabled: config.win_l_enabled,
                    idle_timeout_minutes: config.idle_timeout_minutes,
                    dimming_percentage: config.dimming_percentage,
                    unlock_message: config.unlock_message,
                    hide_taskbar_on_lock: config.hide_taskbar_on_lock,
                    widget: config.widget,
                    unlock_logo_path: config.unlock_logo_path,
                    hotkey: config.hotkey,
                },
                Err(error) => ServiceResponse::Error {
                    message: error.to_string(),
                },
            },
            ClientRequest::Lock => ServiceResponse::Error {
                message: "o comando de bloqueio deve ser enviado ao agente".into(),
            },
        }
    }

    fn verify(&mut self, candidate: &str) -> ServiceResponse {
        let now = Instant::now();
        if let RateLimitDecision::RetryAfter(retry_after_seconds) = self.rate_limiter.check(now) {
            return ServiceResponse::PasswordRejected {
                retry_after_seconds,
            };
        }
        match self.config.verify_unlock_password(candidate) {
            Ok(UnlockPasswordResult::Accepted) => {
                self.rate_limiter.record_success();
                ServiceResponse::PasswordAccepted
            }
            Ok(UnlockPasswordResult::Rejected) => ServiceResponse::PasswordRejected {
                retry_after_seconds: self.rate_limiter.record_failure(now),
            },
            Ok(UnlockPasswordResult::DisabledByWindowsHello) => ServiceResponse::Error {
                message: "a senha do app não pode desbloquear enquanto o Windows Hello está ativo"
                    .into(),
            },
            Err(error) => {
                self.last_error = Some(error.to_string());
                ServiceResponse::Error {
                    message: "configuração de senha inválida".into(),
                }
            }
        }
    }

    fn is_agent_process(&self, client_process: u32) -> bool {
        let expected = (!self.agent_process.is_null())
            .then(|| unsafe { GetProcessId(self.agent_process) })
            .filter(|process_id| *process_id != 0);
        trusted_agent_process(expected, client_process)
    }

    fn require_authentication(&mut self, candidate: &str) -> Result<(), ServiceResponse> {
        match self.authenticate(candidate) {
            Authentication::Accepted => Ok(()),
            Authentication::Rejected {
                retry_after_seconds: 0,
            } => Err(ServiceResponse::Error {
                message: "não foi possível autenticar".into(),
            }),
            Authentication::Rejected {
                retry_after_seconds,
            } => Err(ServiceResponse::Error {
                message: format!("tente novamente em {retry_after_seconds} s"),
            }),
            Authentication::ConfigurationError => Err(ServiceResponse::Error {
                message: "configuração de senha inválida".into(),
            }),
        }
    }

    fn authenticate(&mut self, candidate: &str) -> Authentication {
        let now = Instant::now();
        if let RateLimitDecision::RetryAfter(retry_after_seconds) = self.rate_limiter.check(now) {
            return Authentication::Rejected {
                retry_after_seconds,
            };
        }
        match self.config.verify_password(candidate) {
            Ok(true) => {
                self.rate_limiter.record_success();
                Authentication::Accepted
            }
            Ok(false) => {
                let retry_after_seconds = self.rate_limiter.record_failure(now);
                Authentication::Rejected {
                    retry_after_seconds,
                }
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                Authentication::ConfigurationError
            }
        }
    }
}

enum Authentication {
    Accepted,
    Rejected { retry_after_seconds: u32 },
    ConfigurationError,
}

pub fn dispatch() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("não foi possível iniciar o despachante do serviço")
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        log_event(
            EVENTLOG_ERROR_TYPE,
            &format!("O serviço foi encerrado: {error:#}"),
        );
    }
}

fn run_service() -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop => {
            let _ = event_tx.send(ServiceEvent::Stop);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::SessionChange(change) => {
            let _ = event_tx.send(ServiceEvent::SessionChanged {
                reason: change.reason,
                session_id: change.notification.session_id,
            });
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::PowerEvent(_) => {
            let _ = event_tx.send(ServiceEvent::PowerChanged);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(SERVICE_NAME, handler)?;
    status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP
            | ServiceControlAccept::SESSION_CHANGE
            | ServiceControlAccept::POWER_EVENT,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    })?;

    let store = ConfigStore::new(config_path()?);
    let (enabled, error) = match store.load() {
        Ok(config) => (config.enabled, None),
        Err(error) => (false, Some(error.to_string())),
    };
    let now = Instant::now();
    let runtime = Arc::new(Mutex::new(ServiceRuntime {
        config: store,
        watchdog: Watchdog::new(now),
        enabled,
        agent_running: false,
        locked: false,
        recovery_paused: false,
        shutdown_requested: false,
        last_error: error,
        rate_limiter: RateLimiter::new(),
        agent_process: null_mut(),
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let mut session_id = unsafe { WTSGetActiveConsoleSessionId() };
    let mut session_available = session_id != u32::MAX;
    if session_id != u32::MAX {
        start_control_pipe(session_id, runtime.clone(), stop.clone())?;
        if enabled {
            spawn_agent_into_runtime(&runtime, session_id, false);
        }
    }

    loop {
        if runtime
            .lock()
            .is_ok_and(|runtime| runtime.shutdown_requested)
        {
            break;
        }
        match event_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(ServiceEvent::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(ServiceEvent::SessionChanged {
                reason,
                session_id: changed_session,
            }) => {
                if changed_session == session_id
                    && matches!(
                        reason,
                        SessionChangeReason::SessionLock
                            | SessionChangeReason::SessionLogoff
                            | SessionChangeReason::ConsoleDisconnect
                            | SessionChangeReason::RemoteDisconnect
                            | SessionChangeReason::SessionTerminate
                    )
                {
                    session_available = false;
                    if let Ok(mut runtime) = runtime.lock() {
                        runtime.locked = false;
                        runtime.watchdog.heartbeat(Instant::now(), false);
                    }
                    continue;
                }

                if matches!(
                    reason,
                    SessionChangeReason::SessionUnlock
                        | SessionChangeReason::SessionLogon
                        | SessionChangeReason::ConsoleConnect
                        | SessionChangeReason::RemoteConnect
                ) {
                    let next_session = unsafe { WTSGetActiveConsoleSessionId() };
                    if next_session == u32::MAX {
                        session_available = false;
                        continue;
                    }
                    let changed = next_session != session_id;
                    session_id = next_session;
                    session_available = true;

                    if changed
                        && let Err(error) =
                            start_control_pipe(session_id, runtime.clone(), stop.clone())
                    {
                        log_event(
                            EVENTLOG_ERROR_TYPE,
                            &format!("Falha ao criar canal de controle: {error:#}"),
                        );
                        continue;
                    }

                    let should_start = runtime.lock().is_ok_and(|mut runtime| {
                        runtime.locked = false;
                        runtime.recovery_paused = false;
                        runtime.watchdog.heartbeat(Instant::now(), false);
                        runtime.enabled
                    });
                    if should_start {
                        // Sempre cria uma instância limpa após sair da tela de
                        // bloqueio do Windows. Isso remove qualquer cobertura
                        // transparente que tenha ficado por baixo dela.
                        spawn_agent_into_runtime(&runtime, session_id, false);
                    }
                }
            }
            Ok(ServiceEvent::PowerChanged) => {
                if let Ok(mut runtime) = runtime.lock() {
                    let locked = runtime.locked;
                    runtime.watchdog.heartbeat(Instant::now(), locked);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !session_available {
                    if let Ok(mut runtime) = runtime.lock()
                        && runtime.agent_has_exited()
                    {
                        runtime.terminate_agent();
                        runtime.locked = false;
                        runtime.watchdog.heartbeat(Instant::now(), false);
                    }
                    continue;
                }
                if runtime.lock().is_ok_and(|runtime| runtime.recovery_paused) {
                    continue;
                }
                let process_recovery = runtime.lock().ok().and_then(|mut runtime| {
                    (runtime.enabled && runtime.agent_running && runtime.agent_has_exited()).then(
                        || {
                            runtime.terminate_agent();
                            runtime.watchdog.agent_failed(Instant::now())
                        },
                    )
                });
                if let Some(recovery) = process_recovery {
                    match recovery {
                        WatchdogAction::RestartAgent { locked } => {
                            spawn_agent_into_runtime(&runtime, session_id, locked)
                        }
                        WatchdogAction::LockWindows => {
                            if let Ok(mut runtime) = runtime.lock() {
                                runtime.suspend_recovery_after_windows_fallback(
                                    "A recuperação automática foi pausada após falhas repetidas do agente.",
                                );
                            }
                            log_event(
                                EVENTLOG_WARNING_TYPE,
                                "Falhas repetidas do agente. Aplicando o bloqueio do Windows.",
                            );
                            restore_win_l_in_session(session_id);
                            let _ = spawn_in_session(session_id, "--fallback-lock", false);
                        }
                    }
                    continue;
                }
                let (is_enabled, is_running, was_locked) = runtime
                    .lock()
                    .map(|runtime| (runtime.enabled, runtime.agent_running, runtime.locked))
                    .unwrap_or((false, false, false));
                if is_enabled && !is_running {
                    spawn_agent_into_runtime(&runtime, session_id, was_locked);
                    continue;
                }
                if !is_enabled && is_running {
                    if let Ok(mut runtime) = runtime.lock() {
                        runtime.terminate_agent();
                    }
                    restore_win_l_in_session(session_id);
                    continue;
                }
                let action = runtime.lock().ok().and_then(|mut runtime| {
                    runtime
                        .enabled
                        .then(|| runtime.watchdog.tick(Instant::now()))
                        .flatten()
                });
                if action.is_some() {
                    let recovery = runtime
                        .lock()
                        .map(|mut runtime| runtime.watchdog.agent_failed(Instant::now()))
                        .unwrap_or(WatchdogAction::LockWindows);
                    match recovery {
                        WatchdogAction::RestartAgent { locked } => {
                            spawn_agent_into_runtime(&runtime, session_id, locked)
                        }
                        WatchdogAction::LockWindows => {
                            log_event(
                                EVENTLOG_WARNING_TYPE,
                                "Três falhas do agente em 60 segundos. Aplicando o bloqueio do Windows.",
                            );
                            if let Ok(mut runtime) = runtime.lock() {
                                runtime.suspend_recovery_after_windows_fallback(
                                    "A recuperação automática foi pausada após três falhas do agente.",
                                );
                            }
                            restore_win_l_in_session(session_id);
                            let _ = spawn_in_session(session_id, "--fallback-lock", false);
                        }
                    }
                }
            }
        }
    }

    if let Ok(mut runtime) = runtime.lock() {
        runtime.terminate_agent();
    }
    if session_id != u32::MAX {
        restore_win_l_in_session(session_id);
    }
    terminate_sibling_processes();
    stop.store(true, Ordering::SeqCst);
    let names = PipeNames::for_session(session_id);
    ipc::wake(&names.control);
    if let Ok(mut runtime) = runtime.lock() {
        runtime.terminate_agent();
    }
    status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    })?;
    Ok(())
}

fn terminate_sibling_processes() {
    use std::os::windows::ffi::OsStringExt;

    let Ok(current_executable) = std::env::current_exe() else {
        return;
    };
    let current_path = current_executable.to_string_lossy();
    let current_process = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return;
    }

    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32ProcessID != current_process {
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                    0,
                    entry.th32ProcessID,
                )
            };
            if !process.is_null() {
                let mut path = [0_u16; 32768];
                let mut path_length = path.len() as u32;
                if unsafe {
                    QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut path_length)
                } != 0
                {
                    let candidate = OsString::from_wide(&path[..path_length as usize]);
                    if candidate
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&current_path)
                    {
                        unsafe { TerminateProcess(process, 0) };
                    }
                }
                unsafe { CloseHandle(process) };
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
}

fn start_control_pipe(
    session_id: u32,
    runtime: Arc<Mutex<ServiceRuntime>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let name = PipeNames::for_session(session_id).control;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = ipc::serve(
            name,
            session_id,
            stop,
            Some(ready_tx),
            move |request, client_process| match runtime.lock() {
                Ok(mut runtime) => runtime.handle_request(request, client_process),
                Err(_) => ServiceResponse::Error {
                    message: "estado do serviço indisponível".into(),
                },
            },
        );
    });
    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => bail!("não foi possível reservar o canal de controle: {error}"),
        Err(_) => bail!("tempo esgotado ao iniciar o canal de controle"),
    }
}

fn spawn_agent_into_runtime(runtime: &Arc<Mutex<ServiceRuntime>>, session: u32, locked: bool) {
    if let Ok(mut runtime) = runtime.lock() {
        runtime.terminate_agent();
    }
    restore_win_l_in_session(session);
    if let Ok(mut runtime) = runtime.lock() {
        match spawn_in_session(session, "--agent", locked) {
            Ok(process) => {
                runtime.agent_process = process;
                runtime.agent_running = true;
                runtime.locked = locked;
                runtime.watchdog.heartbeat(Instant::now(), locked);
                runtime.last_error = None;
            }
            Err(error) => {
                runtime.agent_running = false;
                runtime.last_error = Some(error.to_string());
                log_event(
                    EVENTLOG_ERROR_TYPE,
                    &format!("Falha ao iniciar o agente: {error}"),
                );
            }
        }
    }
}

fn restore_win_l_in_session(session: u32) {
    if let Err(error) = configure_win_l_for_session(session, false) {
        log_event(
            EVENTLOG_WARNING_TYPE,
            &format!("Não foi possível restaurar Win + L: {error}"),
        );
    }
}

pub(super) fn restore_win_l_for_current_user() -> Result<()> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        bail!("não foi possível consultar o usuário atual");
    }
    let sid = user_sid_for_token(token);
    unsafe { CloseHandle(token) };
    configure_win_l_for_sid(&sid?, false)
}

fn configure_win_l_for_session(session: u32, enabled: bool) -> Result<()> {
    let sid = user_sid_for_session(session)?;
    configure_win_l_for_sid(&sid, enabled)
}

fn configure_win_l_for_sid(sid: &str, enabled: bool) -> Result<()> {
    let path = wide(&format!(
        r"{sid}\Software\Microsoft\Windows\CurrentVersion\Policies\System"
    ));
    let name = wide("DisableLockWorkstation");
    let backup_name = wide("BloqueioTransparenteOriginalDisableLockWorkstation");
    let mut key = null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_USERS,
            path.as_ptr(),
            0,
            null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("não foi possível abrir a política de Win + L: erro {status}");
    }

    let current = read_registry_dword(key, &name);
    let status = if enabled {
        if read_registry_dword(key, &backup_name).is_none() {
            let backup = current.unwrap_or(2).to_ne_bytes();
            let status = unsafe {
                RegSetValueExW(key, backup_name.as_ptr(), 0, REG_DWORD, backup.as_ptr(), 4)
            };
            if status != ERROR_SUCCESS {
                unsafe { RegCloseKey(key) };
                bail!("não foi possível salvar o estado original de Win + L: erro {status}");
            }
        }
        let value = 1_u32.to_ne_bytes();
        unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_DWORD, value.as_ptr(), 4) }
    } else {
        let saved_original = read_registry_dword(key, &backup_name);
        let status = match crate::windows_policy::win_l_restore_action(saved_original, current) {
            crate::windows_policy::WinLRestoreAction::NoChange => ERROR_SUCCESS,
            crate::windows_policy::WinLRestoreAction::Delete => {
                let result = unsafe { RegDeleteValueW(key, name.as_ptr()) };
                if result == ERROR_FILE_NOT_FOUND {
                    ERROR_SUCCESS
                } else {
                    result
                }
            }
            crate::windows_policy::WinLRestoreAction::Write(original) => {
                let value = original.to_ne_bytes();
                unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_DWORD, value.as_ptr(), 4) }
            }
        };
        if status == ERROR_SUCCESS && saved_original.is_some() {
            unsafe { RegDeleteValueW(key, backup_name.as_ptr()) };
        }
        status
    };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        bail!("não foi possível salvar a política de Win + L: erro {status}");
    }
    Ok(())
}

fn read_registry_dword(key: HKEY, name: &[u16]) -> Option<u32> {
    let mut value = 0_u32;
    let mut value_type = 0_u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            null_mut(),
            &mut value_type,
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS && value_type == REG_DWORD && size == size_of::<u32>() as u32)
        .then_some(value)
}

fn user_sid_for_session(session: u32) -> Result<String> {
    let mut token = null_mut();
    if unsafe { WTSQueryUserToken(session, &mut token) } == 0 {
        bail!("não foi possível consultar o usuário da sessão");
    }
    let sid = user_sid_for_token(token);
    unsafe { CloseHandle(token) };
    sid
}

fn user_sid_for_token(token: HANDLE) -> Result<String> {
    let mut required = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required);
    }
    if required == 0 {
        bail!("não foi possível consultar o SID da sessão");
    }
    let words = (required as usize).div_ceil(size_of::<usize>());
    let mut buffer = vec![0_usize; words];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        bail!("não foi possível ler o SID da sessão");
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_text = null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_text) } == 0 {
        bail!("não foi possível converter o SID da sessão");
    }
    let mut length = 0;
    while unsafe { *sid_text.add(length) } != 0 {
        length += 1;
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) });
    unsafe { LocalFree(sid_text.cast()) };
    sid.context("SID da sessão inválido")
}

fn log_event(event_type: u16, message: &str) {
    unsafe {
        let source_name = wide(super::DISPLAY_NAME);
        let source = RegisterEventSourceW(null(), source_name.as_ptr());
        if source.is_null() {
            return;
        }
        let message = wide(message);
        let strings = [message.as_ptr()];
        ReportEventW(
            source,
            event_type,
            0,
            1,
            null_mut(),
            1,
            0,
            strings.as_ptr(),
            null(),
        );
        DeregisterEventSource(source);
    }
}

fn spawn_in_session(session_id: u32, mode: &str, locked: bool) -> Result<HANDLE> {
    unsafe {
        let mut token = null_mut();
        if WTSQueryUserToken(session_id, &mut token) == 0 {
            bail!(
                "WTSQueryUserToken falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        let executable = std::env::current_exe()?;
        let application = wide_os(executable.as_os_str());
        let mut command = format!("\"{}\" {mode}", executable.display());
        if locked {
            command.push_str(" --locked");
        }
        let mut command = wide(&command);
        let mut desktop = wide("winsta0\\default");
        let mut environment = null_mut();
        if CreateEnvironmentBlock(&mut environment, token, 0) == 0 {
            CloseHandle(token);
            bail!(
                "CreateEnvironmentBlock falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut startup: STARTUPINFOW = zeroed();
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        startup.lpDesktop = desktop.as_mut_ptr();
        let mut process: PROCESS_INFORMATION = zeroed();
        let created = CreateProcessAsUserW(
            token,
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            environment,
            null(),
            &startup,
            &mut process,
        );
        DestroyEnvironmentBlock(environment);
        CloseHandle(token);
        if created == 0 {
            bail!(
                "CreateProcessAsUserW falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        CloseHandle(process.hThread);
        Ok(process.hProcess)
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}
