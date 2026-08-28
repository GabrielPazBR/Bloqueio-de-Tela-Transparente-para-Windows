use super::agent::current_session_id;
use crate::protocol::{ClientRequest, CommandCodec, MAX_FRAME_BYTES, PipeNames, ServiceResponse};
use anyhow::{Context, Result, bail};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, SetFileSecurityW,
};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;

pub fn send_current_session(request: &ClientRequest) -> Result<ServiceResponse> {
    let names = PipeNames::for_session(current_session_id()?);
    let pipe = if matches!(request, ClientRequest::Lock) {
        names.agent
    } else {
        names.control
    };
    send(&pipe, request)
}

pub fn send(pipe_name: &str, request: &ClientRequest) -> Result<ServiceResponse> {
    let name = wide(pipe_name);
    unsafe {
        if WaitNamedPipeW(name.as_ptr(), 3000) == 0 {
            bail!("canal indisponível: {}", std::io::Error::last_os_error());
        }
        let handle = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            bail!(
                "CreateFileW do canal falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        let result = exchange(handle, request);
        CloseHandle(handle);
        result
    }
}

fn exchange(handle: HANDLE, request: &ClientRequest) -> Result<ServiceResponse> {
    unsafe {
        let mode = PIPE_READMODE_MESSAGE;
        if SetNamedPipeHandleState(handle, &mode, null(), null()) == 0 {
            bail!(
                "SetNamedPipeHandleState falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut frame = CommandCodec::encode_request(request)?;
        let mut written = 0;
        if WriteFile(
            handle,
            frame.as_ptr(),
            frame.len() as u32,
            &mut written,
            null_mut(),
        ) == 0
            || written as usize != frame.len()
        {
            frame.fill(0);
            bail!(
                "WriteFile do canal falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        frame.fill(0);
        let mut response = vec![0_u8; MAX_FRAME_BYTES + 4];
        let mut read = 0;
        if ReadFile(
            handle,
            response.as_mut_ptr(),
            response.len() as u32,
            &mut read,
            null_mut(),
        ) == 0
        {
            bail!(
                "ReadFile do canal falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        response.truncate(read as usize);
        CommandCodec::decode_response(&response).context("resposta inválida do serviço")
    }
}

pub fn serve<F>(
    pipe_name: String,
    allowed_session: u32,
    stop: Arc<AtomicBool>,
    ready: Option<std::sync::mpsc::SyncSender<Result<(), String>>>,
    handler: F,
) -> Result<()>
where
    F: Fn(ClientRequest, u32) -> ServiceResponse,
{
    let name = wide(&pipe_name);
    let pipe_acl = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)")?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: pipe_acl.pointer.cast(),
        bInheritHandle: 0,
    };
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            (MAX_FRAME_BYTES + 4) as u32,
            (MAX_FRAME_BYTES + 4) as u32,
            1000,
            &attributes,
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        if let Some(sender) = ready {
            let _ = sender.send(Err(std::io::Error::last_os_error().to_string()));
        }
        bail!(
            "CreateNamedPipeW falhou: {}",
            std::io::Error::last_os_error()
        );
    }
    if let Some(sender) = ready {
        let _ = sender.send(Ok(()));
    }
    while !stop.load(Ordering::Relaxed) {
        let connected = unsafe { ConnectNamedPipe(pipe, null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }
        let _ = serve_connection(pipe, allowed_session, &handler);
        unsafe {
            FlushFileBuffers(pipe);
            DisconnectNamedPipe(pipe);
        }
    }
    unsafe { CloseHandle(pipe) };
    Ok(())
}

pub fn protect_config_file(path: &std::path::Path) -> Result<()> {
    let descriptor = SecurityDescriptor::from_sddl("D:P(A;;FA;;;SY)(A;;FA;;;BA)")?;
    let path = wide_os(path.as_os_str());
    let information = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    if unsafe { SetFileSecurityW(path.as_ptr(), information, descriptor.pointer) } == 0 {
        bail!(
            "SetFileSecurityW falhou: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

struct SecurityDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
}

impl SecurityDescriptor {
    fn from_sddl(value: &str) -> Result<Self> {
        let value = wide(value);
        let mut pointer = null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                value.as_ptr(),
                SDDL_REVISION_1,
                &mut pointer,
                null_mut(),
            )
        } == 0
        {
            bail!(
                "descritor de segurança inválido: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self { pointer })
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.pointer as HLOCAL) };
    }
}

fn serve_connection<F>(pipe: HANDLE, allowed_session: u32, handler: &F) -> Result<()>
where
    F: Fn(ClientRequest, u32) -> ServiceResponse,
{
    unsafe {
        let mut client_process = 0;
        let mut client_session = u32::MAX;
        if GetNamedPipeClientProcessId(pipe, &mut client_process) == 0
            || ProcessIdToSessionId(client_process, &mut client_session) == 0
            || client_session != allowed_session
        {
            bail!("cliente de outra sessão rejeitado");
        }
        let mut frame = vec![0_u8; MAX_FRAME_BYTES + 4];
        let mut read = 0;
        if ReadFile(
            pipe,
            frame.as_mut_ptr(),
            frame.len() as u32,
            &mut read,
            null_mut(),
        ) == 0
        {
            bail!("leitura do canal falhou");
        }
        frame.truncate(read as usize);
        let request = CommandCodec::decode_request(&frame)?;
        frame.fill(0);
        let response = handler(request, client_process);
        let encoded = CommandCodec::encode_response(&response)?;
        let mut written = 0;
        if WriteFile(
            pipe,
            encoded.as_ptr(),
            encoded.len() as u32,
            &mut written,
            null_mut(),
        ) == 0
            || written as usize != encoded.len()
        {
            bail!("escrita do canal falhou");
        }
        Ok(())
    }
}

pub fn wake(pipe_name: &str) {
    let _ = send(pipe_name, &ClientRequest::Status);
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}
