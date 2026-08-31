use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::System::WinRT::{
    IUserConsentVerifierInterop, RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize,
    RoUninitialize,
};
use windows::core::{HSTRING, Interface};
use windows_future::{AsyncStatus, IAsyncInfo, IAsyncOperation};

const RUNTIME_CLASS: &str = "Windows.Security.Credentials.UI.UserConsentVerifier";

#[derive(Debug)]
pub enum VerificationOutcome {
    Verified,
    Canceled,
    Rejected(String),
}

pub struct VerificationCancellation {
    sender: Option<Sender<()>>,
    requested: Arc<AtomicBool>,
}

impl VerificationCancellation {
    pub fn cancel(&mut self) {
        self.requested.store(true, Ordering::Release);
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(());
        }
    }
}

struct WinRtApartment {
    initialized_here: bool,
}

impl WinRtApartment {
    fn initialize() -> Result<Self> {
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Ok(Self {
                initialized_here: true,
            }),
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(Self {
                initialized_here: false,
            }),
            Err(error) => Err(error).context("não foi possível inicializar o Windows Hello"),
        }
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.initialized_here {
            unsafe { RoUninitialize() };
        }
    }
}

pub fn availability() -> Result<UserConsentVerifierAvailability> {
    let _apartment = WinRtApartment::initialize()?;
    UserConsentVerifier::CheckAvailabilityAsync()
        .context("não foi possível consultar o Windows Hello")?
        .get()
        .context("não foi possível consultar o Windows Hello")
}

fn verify_for_window_cancelable(
    owner: HWND,
    message: &str,
    cancellation: Receiver<()>,
) -> Result<UserConsentVerificationResult> {
    let _apartment = WinRtApartment::initialize()?;
    let factory: IUserConsentVerifierInterop = unsafe {
        RoGetActivationFactory(&HSTRING::from(RUNTIME_CLASS))
            .context("não foi possível abrir o Windows Hello")?
    };
    let operation: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        factory
            .RequestVerificationForWindowAsync(owner, &HSTRING::from(message))
            .context("não foi possível abrir o Windows Hello")?
    };
    let async_info: IAsyncInfo = operation
        .cast()
        .context("não foi possível controlar a verificação do Windows Hello")?;
    let mut cancellation_connected = true;
    while async_info
        .Status()
        .context("não foi possível consultar o Windows Hello")?
        == AsyncStatus::Started
    {
        if cancellation_connected {
            match cancellation.recv_timeout(Duration::from_millis(25)) {
                Ok(()) => {
                    async_info
                        .Cancel()
                        .context("não foi possível cancelar o Windows Hello")?;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => cancellation_connected = false,
            }
        } else {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    operation
        .get()
        .context("a verificação do Windows Hello falhou")
}

pub fn verify_for_window_async<F>(
    owner: isize,
    message: String,
    complete: F,
) -> VerificationCancellation
where
    F: FnOnce(VerificationOutcome) + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let requested = Arc::new(AtomicBool::new(false));
    let worker_requested = requested.clone();
    crate::background::run(
        move || {
            let owner = HWND(owner as *mut std::ffi::c_void);
            match verify_for_window_cancelable(owner, &message, receiver) {
                Ok(UserConsentVerificationResult::Verified) => VerificationOutcome::Verified,
                Ok(UserConsentVerificationResult::Canceled) => VerificationOutcome::Canceled,
                Ok(value) => VerificationOutcome::Rejected(verification_message(value).to_owned()),
                Err(_) if worker_requested.load(Ordering::Acquire) => VerificationOutcome::Canceled,
                Err(error) => VerificationOutcome::Rejected(error.to_string()),
            }
        },
        complete,
    );
    VerificationCancellation {
        sender: Some(sender),
        requested,
    }
}

pub fn verify_activation_for_window_async<F>(
    owner: isize,
    message: String,
    complete: F,
) -> VerificationCancellation
where
    F: FnOnce(VerificationOutcome) + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let requested = Arc::new(AtomicBool::new(false));
    let worker_requested = requested.clone();
    crate::background::run(
        move || {
            let availability = match availability() {
                Ok(value) => value,
                Err(error) => return VerificationOutcome::Rejected(error.to_string()),
            };
            if availability != UserConsentVerifierAvailability::Available {
                return VerificationOutcome::Rejected(
                    availability_message(availability).to_owned(),
                );
            }
            if worker_requested.load(Ordering::Acquire) {
                return VerificationOutcome::Canceled;
            }
            let owner = HWND(owner as *mut std::ffi::c_void);
            match verify_for_window_cancelable(owner, &message, receiver) {
                Ok(UserConsentVerificationResult::Verified) => VerificationOutcome::Verified,
                Ok(UserConsentVerificationResult::Canceled) => VerificationOutcome::Canceled,
                Ok(value) => VerificationOutcome::Rejected(verification_message(value).to_owned()),
                Err(_) if worker_requested.load(Ordering::Acquire) => VerificationOutcome::Canceled,
                Err(error) => VerificationOutcome::Rejected(error.to_string()),
            }
        },
        complete,
    );
    VerificationCancellation {
        sender: Some(sender),
        requested,
    }
}

pub fn availability_message(value: UserConsentVerifierAvailability) -> &'static str {
    match value {
        UserConsentVerifierAvailability::Available => "Windows Hello disponível",
        UserConsentVerifierAvailability::DeviceNotPresent => {
            "Este dispositivo não oferece Windows Hello"
        }
        UserConsentVerifierAvailability::NotConfiguredForUser => {
            "Configure o Windows Hello nas configurações do Windows"
        }
        UserConsentVerifierAvailability::DisabledByPolicy => {
            "O Windows Hello está desativado pela política do Windows"
        }
        UserConsentVerifierAvailability::DeviceBusy => "O Windows Hello está ocupado",
        _ => "O Windows Hello não está disponível",
    }
}

pub fn verification_message(value: UserConsentVerificationResult) -> &'static str {
    match value {
        UserConsentVerificationResult::Verified => "Identidade confirmada",
        UserConsentVerificationResult::Canceled => "Verificação cancelada",
        UserConsentVerificationResult::DeviceNotPresent => {
            "Este dispositivo não oferece Windows Hello"
        }
        UserConsentVerificationResult::NotConfiguredForUser => {
            "Configure o Windows Hello nas configurações do Windows"
        }
        UserConsentVerificationResult::DisabledByPolicy => {
            "O Windows Hello está desativado pela política do Windows"
        }
        UserConsentVerificationResult::DeviceBusy => "O Windows Hello está ocupado",
        UserConsentVerificationResult::RetriesExhausted => {
            "O limite de tentativas do Windows Hello foi atingido"
        }
        _ => "O Windows Hello não confirmou sua identidade",
    }
}
