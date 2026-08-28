use super::{DISPLAY_NAME, ipc};
use crate::lock::{Action, Event, LockController, LockState};
use crate::protocol::{ClientRequest, ServiceResponse};
use crate::windows_policy::{
    ClockWidgetLayout, ImageLayout, KeyDecision, KeyEvent, MonitorRect, OverlayLayout, VirtualKey,
    clock_date_label, dimming_alpha,
};
use anyhow::{Context, Result, bail};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::BOOL;

const CLASS_NAME: &str = "BloqueioTransparente.Window";
const HOTKEY_ID: i32 = 0x4254;
const TIMER_ID: usize = 1;
const WM_TRAY: u32 = WM_APP + 1;
const WM_LOCK_REQUEST: u32 = WM_APP + 2;
const MENU_LOCK: usize = 1001;
const MENU_SETTINGS: usize = 1002;
const MENU_STATUS: usize = 1003;

static RUNTIME: OnceLock<Mutex<AgentRuntime>> = OnceLock::new();
static LOCKED: AtomicBool = AtomicBool::new(false);
static MANAGER_WINDOW: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED_MESSAGE: AtomicU32 = AtomicU32::new(0);

struct AgentRuntime {
    controller: LockController,
    manager: HWND,
    overlays: Vec<HWND>,
    prompt: HWND,
    hooks: Option<HookThread>,
    last_heartbeat: Instant,
    unlock_message: String,
    widget: crate::config::WidgetConfig,
    unlock_logo_path: Option<String>,
    hide_taskbar_on_lock: bool,
    hidden_taskbars: Vec<HWND>,
}

// Os HWND pertencem à thread de UI. O mutex existe apenas para permitir que os
// callbacks Win32 consultem o estado; nenhuma outra thread manipula as janelas.
unsafe impl Send for AgentRuntime {}

struct HookThread {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl Drop for HookThread {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn run(start_locked: bool) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(null());
        if instance.is_null() {
            bail!(
                "GetModuleHandleW falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        let class_name = wide(CLASS_NAME);
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            bail!("RegisterClassW falhou: {}", std::io::Error::last_os_error());
        }

        let title = wide(DISPLAY_NAME);
        let manager = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if manager.is_null() {
            bail!(
                "não foi possível criar a janela do agente: {}",
                std::io::Error::last_os_error()
            );
        }
        MANAGER_WINDOW.store(manager as isize, Ordering::SeqCst);
        let taskbar_created = wide("TaskbarCreated");
        TASKBAR_CREATED_MESSAGE.store(
            RegisterWindowMessageW(taskbar_created.as_ptr()),
            Ordering::Release,
        );
        let runtime = AgentRuntime {
            controller: LockController::new(),
            manager,
            overlays: Vec::new(),
            prompt: null_mut(),
            hooks: None,
            last_heartbeat: Instant::now(),
            unlock_message: crate::config::default_unlock_message(),
            widget: crate::config::WidgetConfig::default(),
            unlock_logo_path: None,
            hide_taskbar_on_lock: false,
            hidden_taskbars: Vec::new(),
        };
        RUNTIME
            .set(Mutex::new(runtime))
            .map_err(|_| anyhow::anyhow!("agente já inicializado"))?;

        register_hotkey(manager)?;
        add_tray_icon(manager)?;
        start_agent_pipe(manager)?;
        // A integração com Win + L está suspensa. Restaura a política normal
        // para instalações que tinham a opção ativa em versões anteriores.
        let _ = configure_win_l_override(false);
        SetTimer(manager, TIMER_ID, 1000, None);
        // Recupera a barra de tarefas caso uma instância anterior tenha sido
        // encerrada enquanto o Windows exibia a área de trabalho segura.
        show_all_taskbars();
        if start_locked {
            request_lock()?;
        }

        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        delete_tray_icon(manager);
        UnregisterHotKey(manager, HOTKEY_ID);
        let _ = configure_win_l_override(false);
        Ok(())
    }
}

pub fn lock_windows() -> Result<()> {
    let _ = configure_win_l_override(false);
    show_all_taskbars();
    unsafe {
        if LockWorkStation() == 0 {
            bail!(
                "LockWorkStation falhou: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}

pub fn current_session_id() -> Result<u32> {
    unsafe {
        let mut session = 0;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut session) == 0 {
            bail!(
                "ProcessIdToSessionId falhou: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(session)
    }
}

pub fn request_lock() -> Result<()> {
    let runtime = RUNTIME.get().context("agente não inicializado")?;
    let actions = runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("estado do agente indisponível"))?
        .controller
        .handle(Event::LockRequested, Instant::now());
    if let Err(error) = apply_actions(actions) {
        activate_windows_fallback().context("falha ao aplicar o bloqueio e a recuperação")?;
        return Err(error.context("falha ao aplicar o bloqueio transparente"));
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if crate::windows_policy::should_restore_tray_icon(
        message,
        TASKBAR_CREATED_MESSAGE.load(Ordering::Acquire),
    ) {
        let _ = add_tray_icon(window);
        return 0;
    }
    match message {
        WM_HOTKEY if wparam as i32 == HOTKEY_ID => {
            let _ = request_lock();
            0
        }
        WM_LOCK_REQUEST => {
            let _ = request_lock();
            0
        }
        WM_CHAR => {
            handle_character(wparam as u32);
            0
        }
        WM_PAINT => {
            paint_window(window);
            0
        }
        WM_DISPLAYCHANGE => {
            handle_event(Event::DisplayChanged);
            0
        }
        WM_TIMER => {
            timer_tick();
            0
        }
        WM_COMMAND => {
            handle_menu(wparam & 0xffff);
            0
        }
        WM_TRAY => {
            match crate::windows_policy::tray_action(lparam as u32) {
                crate::windows_policy::TrayAction::OpenSettings => {
                    let _ = spawn_cli("settings");
                }
                crate::windows_policy::TrayAction::OpenMenu => show_tray_menu(window),
                crate::windows_policy::TrayAction::Ignore => {}
            }
            0
        }
        WM_CLOSE if LOCKED.load(Ordering::SeqCst) => 0,
        WM_DESTROY => {
            if crate::windows_policy::should_quit_on_window_destroy(
                window as isize,
                MANAGER_WINDOW.load(Ordering::Acquire),
            ) {
                MANAGER_WINDOW.store(0, Ordering::Release);
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn handle_character(value: u32) {
    match value {
        8 => handle_event(Event::Backspace),
        13 => handle_event(Event::SubmitPassword),
        27 => handle_event(Event::CancelPrompt),
        value if value >= 0x20 => {
            if let Some(character) = char::from_u32(value) {
                handle_event(Event::PrintableCharacter(character));
            }
        }
        _ => {}
    }
    invalidate_prompt();
}

fn handle_event(event: Event) {
    let Some(runtime) = RUNTIME.get() else { return };
    let actions = match runtime.lock() {
        Ok(mut runtime) => runtime.controller.handle(event, Instant::now()),
        Err(_) => return,
    };
    if apply_actions(actions).is_err() && LOCKED.load(Ordering::SeqCst) {
        let _ = activate_windows_fallback();
    }
}

fn apply_actions(actions: Vec<Action>) -> Result<()> {
    for action in actions {
        match action {
            Action::ShowOverlays => create_overlays()?,
            Action::HideOverlays => destroy_overlays(),
            Action::RebuildOverlays => rebuild_overlays()?,
            Action::InstallInputHooks => install_hooks()?,
            Action::RemoveInputHooks => remove_hooks(),
            Action::ShowPasswordPrompt => show_prompt()?,
            Action::HidePasswordPrompt => hide_prompt(),
            Action::ShowPasswordError => invalidate_prompt(),
            Action::VerifyPassword(candidate) => {
                invalidate_prompt();
                if let Some(runtime) = RUNTIME.get()
                    && let Ok(runtime) = runtime.lock()
                    && !runtime.prompt.is_null()
                {
                    unsafe { UpdateWindow(runtime.prompt) };
                }
                verify_password(&candidate);
            }
            Action::LockWindows => {
                activate_windows_fallback()?;
            }
        }
    }
    Ok(())
}

fn activate_windows_fallback() -> Result<()> {
    lock_windows()?;
    remove_hooks();
    destroy_overlays();
    Ok(())
}

fn verify_password(candidate: &str) {
    let request = crate::protocol::ClientRequest::VerifyPassword {
        candidate: candidate.into(),
    };
    match ipc::send_current_session(&request) {
        Ok(crate::protocol::ServiceResponse::PasswordAccepted) => {
            handle_event(Event::PasswordAccepted)
        }
        Ok(crate::protocol::ServiceResponse::PasswordRejected {
            retry_after_seconds,
        }) => handle_event(Event::PasswordRejected {
            retry_after_seconds,
        }),
        _ => handle_event(Event::ConfigurationCorrupt),
    }
}

fn build_overlay_windows() -> Result<Vec<HWND>> {
    let dimming_percentage = match ipc::send_current_session(&ClientRequest::Settings) {
        Ok(ServiceResponse::Settings {
            dimming_percentage, ..
        }) => dimming_percentage,
        _ => 0,
    };
    let alpha = dimming_alpha(dimming_percentage);
    let monitors = enumerate_monitors()?;
    let layouts = OverlayLayout::from_monitors(&monitors);
    let class_name = wide(CLASS_NAME);
    let title = wide(DISPLAY_NAME);
    let instance = unsafe { GetModuleHandleW(null()) };
    let mut windows = Vec::new();
    for layout in layouts {
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                layout.x,
                layout.y,
                layout.width,
                layout.height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if window.is_null() {
            destroy_window_list(&windows);
            bail!("não foi possível criar uma cobertura de monitor");
        }
        unsafe {
            SetLayeredWindowAttributes(window, 0, alpha, LWA_ALPHA);
            ShowWindow(window, SW_SHOW);
            SetWindowPos(
                window,
                HWND_TOPMOST,
                layout.x,
                layout.y,
                layout.width,
                layout.height,
                SWP_SHOWWINDOW,
            );
        }
        windows.push(window);
    }
    Ok(windows)
}

fn create_overlays() -> Result<()> {
    let (widget, hide_taskbar_on_lock, unlock_logo_path) =
        match ipc::send_current_session(&ClientRequest::Settings) {
            Ok(ServiceResponse::Settings {
                widget,
                hide_taskbar_on_lock,
                unlock_logo_path,
                ..
            }) => (widget, hide_taskbar_on_lock, unlock_logo_path),
            _ => (crate::config::WidgetConfig::default(), false, None),
        };
    let windows = build_overlay_windows()?;
    let runtime = RUNTIME.get().context("agente não inicializado")?;
    let mut runtime = runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("estado indisponível"))?;
    let previous = std::mem::replace(&mut runtime.overlays, windows);
    runtime.widget = widget;
    runtime.hide_taskbar_on_lock = hide_taskbar_on_lock;
    runtime.unlock_logo_path = unlock_logo_path;
    destroy_window_list(&previous);
    restore_taskbars(&mut runtime.hidden_taskbars);
    if runtime.hide_taskbar_on_lock {
        runtime.hidden_taskbars = hide_taskbars();
    }
    if let Some(&first) = runtime.overlays.first() {
        unsafe {
            SetForegroundWindow(first);
            SetFocus(first);
        }
    }
    LOCKED.store(true, Ordering::SeqCst);
    Ok(())
}

fn rebuild_overlays() -> Result<()> {
    let windows = build_overlay_windows()?;
    let runtime = RUNTIME.get().context("agente não inicializado")?;
    let mut runtime = runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("estado indisponível"))?;
    let previous = std::mem::replace(&mut runtime.overlays, windows);
    destroy_window_list(&previous);
    let target = if runtime.prompt.is_null() {
        runtime.overlays.first().copied()
    } else {
        Some(runtime.prompt)
    };
    if let Some(target) = target {
        unsafe {
            SetWindowPos(
                target,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            SetForegroundWindow(target);
            SetFocus(target);
        }
    }
    LOCKED.store(true, Ordering::SeqCst);
    Ok(())
}

fn destroy_overlays() {
    LOCKED.store(false, Ordering::SeqCst);
    if let Some(runtime) = RUNTIME.get()
        && let Ok(mut runtime) = runtime.lock()
    {
        destroy_window_list(&runtime.overlays);
        runtime.overlays.clear();
        restore_taskbars(&mut runtime.hidden_taskbars);
        if !runtime.prompt.is_null() {
            unsafe { DestroyWindow(runtime.prompt) };
            runtime.prompt = null_mut();
        }
    }
    let _ = ipc::send_current_session(&ClientRequest::Heartbeat { locked: false });
}

fn destroy_window_list(windows: &[HWND]) {
    for &window in windows {
        unsafe { DestroyWindow(window) };
    }
}

fn hide_taskbars() -> Vec<HWND> {
    unsafe extern "system" fn callback(window: HWND, data: LPARAM) -> BOOL {
        let mut class_name = [0_u16; 64];
        let length =
            unsafe { GetClassNameW(window, class_name.as_mut_ptr(), class_name.len() as i32) };
        if length > 0 {
            let name = String::from_utf16_lossy(&class_name[..length as usize]);
            if (name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd")
                && unsafe { IsWindowVisible(window) } != 0
            {
                unsafe { ShowWindow(window, SW_HIDE) };
                unsafe { (&mut *(data as *mut Vec<HWND>)).push(window) };
            }
        }
        1
    }
    let mut windows = Vec::new();
    unsafe { EnumWindows(Some(callback), &mut windows as *mut _ as LPARAM) };
    windows
}

fn restore_taskbars(windows: &mut Vec<HWND>) {
    for window in windows.drain(..) {
        unsafe { ShowWindow(window, SW_SHOW) };
    }
}

fn show_all_taskbars() {
    unsafe extern "system" fn callback(window: HWND, _data: LPARAM) -> BOOL {
        let mut class_name = [0_u16; 64];
        let length =
            unsafe { GetClassNameW(window, class_name.as_mut_ptr(), class_name.len() as i32) };
        if length > 0 {
            let name = String::from_utf16_lossy(&class_name[..length as usize]);
            if name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd" {
                unsafe { ShowWindow(window, SW_SHOW) };
            }
        }
        1
    }
    unsafe { EnumWindows(Some(callback), 0) };
}

fn show_prompt() -> Result<()> {
    let (unlock_message, unlock_logo_path) =
        match ipc::send_current_session(&ClientRequest::Settings) {
            Ok(ServiceResponse::Settings {
                unlock_message,
                unlock_logo_path,
                ..
            }) => (unlock_message, unlock_logo_path),
            _ => (crate::config::default_unlock_message(), None),
        };
    let runtime = RUNTIME.get().context("agente não inicializado")?;
    let mut runtime = runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("estado indisponível"))?;
    runtime.unlock_message = unlock_message;
    runtime.unlock_logo_path = unlock_logo_path;
    if runtime.prompt.is_null() {
        let class_name = wide(CLASS_NAME);
        let title = wide(&runtime.unlock_message);
        let width = 640;
        let height = if runtime.unlock_logo_path.is_some() {
            380
        } else {
            330
        };
        let x = unsafe { (GetSystemMetrics(SM_CXSCREEN) - width) / 2 };
        let y = unsafe { (GetSystemMetrics(SM_CYSCREEN) - height) / 2 };
        runtime.prompt = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if runtime.prompt.is_null() {
            bail!("não foi possível criar o campo de senha");
        }
        let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, 24, 24) };
        if !region.is_null() && unsafe { SetWindowRgn(runtime.prompt, region, 1) } == 0 {
            unsafe { DeleteObject(region) };
        }
    }
    unsafe {
        ShowWindow(runtime.prompt, SW_SHOW);
        SetWindowPos(
            runtime.prompt,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(runtime.prompt);
        SetFocus(runtime.prompt);
        InvalidateRect(runtime.prompt, null(), 1);
    }
    Ok(())
}

fn hide_prompt() {
    if let Some(runtime) = RUNTIME.get()
        && let Ok(runtime) = runtime.lock()
        && !runtime.prompt.is_null()
    {
        unsafe { ShowWindow(runtime.prompt, SW_HIDE) };
        if let Some(&first) = runtime.overlays.first() {
            unsafe { SetForegroundWindow(first) };
        }
    }
}

fn invalidate_prompt() {
    if let Some(runtime) = RUNTIME.get()
        && let Ok(runtime) = runtime.lock()
        && !runtime.prompt.is_null()
    {
        unsafe { InvalidateRect(runtime.prompt, null(), 1) };
    }
}

fn paint_window(window: HWND) {
    let mut paint: PAINTSTRUCT = unsafe { zeroed() };
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if dc.is_null() {
        return;
    }
    let appearance = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.prompt == window,
                runtime.widget.clone(),
                runtime.unlock_logo_path.clone(),
            )
        });
    let is_prompt = appearance.as_ref().is_some_and(|value| value.0);
    if is_prompt {
        let logo_path = appearance.as_ref().and_then(|value| value.2.as_deref());
        paint_unlock_prompt(dc, logo_path);
        unsafe { EndPaint(window, &paint) };
        return;
    }
    if is_prompt {
        let mut rect: RECT = unsafe { zeroed() };
        unsafe {
            GetClientRect(window, &mut rect);
            SetBkColor(dc, 0x00202020);
            SetTextColor(dc, 0x00ffffff);
            let brush = CreateSolidBrush(0x00202020);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
        }
        let (message, count, failed, retry_after) = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .map(|runtime| {
                (
                    runtime.unlock_message.clone(),
                    runtime.controller.password_buffer().chars().count(),
                    runtime.controller.failed_attempts() > 0,
                    runtime.controller.retry_at().map(|deadline| {
                        deadline
                            .saturating_duration_since(Instant::now())
                            .as_secs()
                            .max(1)
                    }),
                )
            })
            .unwrap_or_default();
        let logo_path = appearance.as_ref().and_then(|value| value.2.as_deref());
        if let Some(path) = logo_path {
            draw_image(dc, path, 232, 12, 96, 64);
        }
        let label = wide(&message);
        let bullets = wide(&"●".repeat(count));
        unsafe {
            let mut label_rect = RECT {
                left: 24,
                top: if logo_path.is_some() { 82 } else { 20 },
                right: 536,
                bottom: 74,
            };
            DrawTextW(
                dc,
                label.as_ptr(),
                (label.len() - 1) as i32,
                &mut label_rect,
                DT_CENTER | DT_WORDBREAK,
            );
            let field_y = if logo_path.is_some() { 148 } else { 86 };
            TextOutW(
                dc,
                24,
                field_y,
                bullets.as_ptr(),
                (bullets.len() - 1) as i32,
            );
            if failed {
                let error = wide(&retry_after.map_or_else(
                    || "Senha incorreta".into(),
                    |seconds| format!("Tente novamente em {seconds} s"),
                ));
                SetTextColor(dc, 0x006060ff);
                TextOutW(
                    dc,
                    24,
                    field_y + 46,
                    error.as_ptr(),
                    (error.len() - 1) as i32,
                );
            }
        }
    } else if let Some((_, widget, _)) = appearance {
        paint_widget(window, dc, &widget);
    }
    unsafe { EndPaint(window, &paint) };
}

fn paint_unlock_prompt(dc: HDC, logo_path: Option<&str>) {
    let mut rect: RECT = unsafe { zeroed() };
    unsafe { GetClipBox(dc, &mut rect) };
    let width = rect.right - rect.left;
    let background = rgb(20, 22, 25);
    let surface = rgb(34, 37, 41);
    let text = rgb(242, 244, 247);
    let muted = rgb(176, 183, 193);
    let primary = rgb(105, 174, 235);
    let error_color = rgb(244, 113, 116);

    unsafe {
        let brush = CreateSolidBrush(background);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
        let accent = CreateSolidBrush(rgb(34, 102, 164));
        FillRect(
            dc,
            &RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: 5,
            },
            accent,
        );
        DeleteObject(accent);
    }

    if let Some(path) = logo_path {
        draw_image(dc, path, width / 2 - 80, 22, 160, 72);
    } else {
        unsafe {
            let instance = GetModuleHandleW(null());
            let icon = LoadIconW(instance, std::ptr::without_provenance(1));
            if !icon.is_null() {
                DrawIconEx(
                    dc,
                    width / 2 - 24,
                    24,
                    icon,
                    48,
                    48,
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
        }
    }

    let (message, count, failed, retry_after, state) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.unlock_message.clone(),
                runtime.controller.password_buffer().chars().count(),
                runtime.controller.failed_attempts() > 0,
                runtime.controller.retry_at().map(|deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs()
                        .max(1)
                }),
                runtime.controller.state(),
            )
        })
        .unwrap_or_else(|| (String::new(), 0, false, None, LockState::Prompting));
    let heading_top = if logo_path.is_some() { 108 } else { 84 };
    draw_gdi_text(
        dc,
        "Tela bloqueada",
        RECT {
            left: 48,
            top: heading_top,
            right: width - 48,
            bottom: heading_top + 34,
        },
        24,
        FW_SEMIBOLD as i32,
        text,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    draw_gdi_text(
        dc,
        &message,
        RECT {
            left: 56,
            top: heading_top + 36,
            right: width - 56,
            bottom: heading_top + 82,
        },
        15,
        FW_NORMAL as i32,
        muted,
        DT_CENTER | DT_VCENTER | DT_WORDBREAK,
    );

    let field_top = heading_top + 108;
    draw_gdi_text(
        dc,
        "Senha",
        RECT {
            left: 64,
            top: field_top - 26,
            right: width - 64,
            bottom: field_top - 4,
        },
        14,
        FW_SEMIBOLD as i32,
        text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let field_border = if failed || retry_after.is_some() {
        error_color
    } else {
        primary
    };
    draw_round_panel(
        dc,
        RECT {
            left: 64,
            top: field_top,
            right: width - 64,
            bottom: field_top + 50,
        },
        surface,
        field_border,
        12,
    );
    let visible = count.min(24);
    let mut bullets = "\u{2022}".repeat(visible);
    if count > visible {
        bullets.push('\u{2026}');
    }
    draw_gdi_text(
        dc,
        &bullets,
        RECT {
            left: 82,
            top: field_top,
            right: width - 82,
            bottom: field_top + 50,
        },
        22,
        FW_NORMAL as i32,
        text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    let (status, status_color) = if let Some(seconds) = retry_after {
        (format!("Tente novamente em {seconds} s"), error_color)
    } else if state == LockState::Verifying {
        ("Verificando senha...".to_owned(), primary)
    } else if failed {
        ("Senha incorreta. Tente novamente.".to_owned(), error_color)
    } else {
        ("Pressione Enter para desbloquear".to_owned(), muted)
    };
    draw_gdi_text(
        dc,
        &status,
        RECT {
            left: 64,
            top: field_top + 60,
            right: width - 64,
            bottom: field_top + 86,
        },
        14,
        FW_NORMAL as i32,
        status_color,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    draw_gdi_text(
        dc,
        "Esc oculta este campo",
        RECT {
            left: 64,
            top: rect.bottom - 32,
            right: width - 64,
            bottom: rect.bottom - 10,
        },
        12,
        FW_NORMAL as i32,
        muted,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

fn draw_round_panel(dc: HDC, rect: RECT, fill: COLORREF, border: COLORREF, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 2, border);
        let previous_brush = SelectObject(dc, brush);
        let previous_pen = SelectObject(dc, pen);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, previous_brush);
        SelectObject(dc, previous_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gdi_text(
    dc: HDC,
    value: &str,
    mut rect: RECT,
    size: i32,
    weight: i32,
    color: COLORREF,
    format: u32,
) {
    let value = wide(value);
    let face = wide("Segoe UI");
    unsafe {
        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        );
        let previous_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            value.as_ptr(),
            (value.len() - 1) as i32,
            &mut rect,
            format,
        );
        SelectObject(dc, previous_font);
        DeleteObject(font);
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn paint_widget(window: HWND, dc: HDC, widget: &crate::config::WidgetConfig) {
    use crate::config::WidgetKind;
    if widget.kind == WidgetKind::None {
        return;
    }
    let mut screen: RECT = unsafe { zeroed() };
    let mut client: RECT = unsafe { zeroed() };
    unsafe {
        GetWindowRect(window, &mut screen);
        GetClientRect(window, &mut client);
    }
    let current_monitor = MonitorRect::new(screen.left, screen.top, screen.right, screen.bottom);
    if enumerate_monitors()
        .ok()
        .and_then(|monitors| crate::windows_policy::central_monitor(&monitors))
        != Some(current_monitor)
    {
        return;
    }
    let layout = crate::windows_policy::WidgetLayout::place(
        MonitorRect::new(0, 0, client.right, client.bottom),
        widget.width as i32,
        widget.height as i32,
        widget.x_percent,
        widget.y_percent,
    );
    match widget.kind {
        WidgetKind::Clock => {
            let mut time: SYSTEMTIME = unsafe { zeroed() };
            unsafe { GetLocalTime(&mut time) };
            let clock = ClockWidgetLayout::from_widget(layout);
            draw_clock_panel(dc, clock);
            draw_clock_text(
                dc,
                &format!("{:02}:{:02}", time.wHour, time.wMinute),
                clock.time,
                clock.time_font_size,
                FW_SEMIBOLD as i32,
                rgb(246, 248, 251),
                "Bahnschrift SemiBold",
            );
            draw_clock_text(
                dc,
                &clock_date_label(time.wDayOfWeek, time.wDay, time.wMonth, time.wYear),
                clock.date,
                clock.date_font_size,
                FW_NORMAL as i32,
                rgb(166, 181, 198),
                "Bahnschrift",
            );
        }
        WidgetKind::Image => {
            if let Some(path) = widget.image_path.as_deref() {
                draw_image(dc, path, layout.x, layout.y, layout.width, layout.height);
            }
        }
        WidgetKind::None => {}
    }
}

fn draw_clock_panel(dc: HDC, layout: ClockWidgetLayout) {
    draw_rounded_panel(
        dc,
        layout.outer,
        layout.corner_radius,
        rgb(12, 15, 20),
        rgb(84, 101, 120),
    );
    draw_rounded_panel(
        dc,
        layout.inner,
        (layout.corner_radius - 4).max(6),
        rgb(23, 28, 35),
        rgb(47, 60, 74),
    );
}

fn draw_rounded_panel(
    dc: HDC,
    layout: crate::windows_policy::WidgetLayout,
    radius: i32,
    fill: COLORREF,
    border: COLORREF,
) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let previous_brush = SelectObject(dc, brush);
        let previous_pen = SelectObject(dc, pen);
        RoundRect(
            dc,
            layout.x,
            layout.y,
            layout.x + layout.width,
            layout.y + layout.height,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clock_text(
    dc: HDC,
    value: &str,
    layout: crate::windows_policy::WidgetLayout,
    size: i32,
    weight: i32,
    color: COLORREF,
    face: &str,
) {
    let value = wide(value);
    let face = wide(face);
    let mut rect = RECT {
        left: layout.x,
        top: layout.y,
        right: layout.x + layout.width,
        bottom: layout.y + layout.height,
    };
    unsafe {
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        );
        let previous_font = SelectObject(dc, font);
        DrawTextW(
            dc,
            value.as_ptr(),
            (value.len() - 1) as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(dc, previous_font);
        DeleteObject(font);
    }
}

fn draw_image(dc: HDC, path: &str, x: i32, y: i32, width: i32, height: i32) {
    let Ok(image) = image::open(path).map(|image| image.to_rgba8()) else {
        return;
    };
    let (source_width, source_height) = image.dimensions();
    let Some(layout) = ImageLayout::contain(x, y, width, height, source_width, source_height)
    else {
        return;
    };
    let mut pixels = image.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = source_width as i32;
    info.bmiHeader.biHeight = -(source_height as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    unsafe {
        StretchDIBits(
            dc,
            layout.x,
            layout.y,
            layout.width,
            layout.height,
            0,
            0,
            source_width as i32,
            source_height as i32,
            pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

fn enumerate_monitors() -> Result<Vec<MonitorRect>> {
    unsafe extern "system" fn callback(
        _monitor: HMONITOR,
        _dc: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        if rect.is_null() || data == 0 {
            return 0;
        }
        let (monitors, rect) = unsafe { (&mut *(data as *mut Vec<MonitorRect>), *rect) };
        monitors.push(MonitorRect::new(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
        ));
        1
    }
    let mut monitors = Vec::new();
    let success = unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(callback),
            &mut monitors as *mut _ as LPARAM,
        )
    };
    if success == 0 || monitors.is_empty() {
        bail!("não foi possível enumerar os monitores");
    }
    Ok(monitors)
}

fn install_hooks() -> Result<()> {
    if RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| runtime.hooks.is_some())
    {
        return Ok(());
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || unsafe {
        let thread_id = windows_sys::Win32::System::Threading::GetCurrentThreadId();
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), null_mut(), 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), null_mut(), 0);
        let _ = ready_tx.send((thread_id, !keyboard.is_null() && !mouse.is_null()));
        if keyboard.is_null() || mouse.is_null() {
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            return;
        }
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(keyboard);
        UnhookWindowsHookEx(mouse);
    });
    let (thread_id, installed) = ready_rx.recv().context("thread de hooks não respondeu")?;
    if !installed {
        let _ = join.join();
        bail!("não foi possível instalar os hooks de entrada");
    }
    let runtime = RUNTIME.get().context("agente não inicializado")?;
    runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("estado indisponível"))?
        .hooks = Some(HookThread {
        thread_id,
        join: Some(join),
    });
    Ok(())
}

fn remove_hooks() {
    if let Some(runtime) = RUNTIME.get()
        && let Ok(mut runtime) = runtime.lock()
    {
        runtime.hooks.take();
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && LOCKED.load(Ordering::Relaxed) {
            1
        } else {
            CallNextHookEx(null_mut(), code, wparam, lparam)
        }
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }
        let data = &*(lparam as *const KBDLLHOOKSTRUCT);
        if !LOCKED.load(Ordering::Relaxed) {
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }
        let key = match data.vkCode {
            key if key == VK_TAB as u32 => VirtualKey::Tab,
            key if key == VK_ESCAPE as u32 => VirtualKey::Escape,
            key if key == VK_LWIN as u32 => VirtualKey::LWin,
            key if key == VK_RWIN as u32 => VirtualKey::RWin,
            key => VirtualKey::Other(key),
        };
        let event = KeyEvent {
            key,
            control: GetAsyncKeyState(VK_CONTROL as i32) < 0,
            alt: data.flags & LLKHF_ALTDOWN != 0,
            shift: GetAsyncKeyState(VK_SHIFT as i32) < 0,
            key_down: wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN,
        };
        let foreground = GetForegroundWindow();
        let mut process_id = 0;
        GetWindowThreadProcessId(foreground, &mut process_id);
        let owns_foreground = process_id == GetCurrentProcessId();
        match event.decision(true, owns_foreground) {
            KeyDecision::Consume => 1,
            KeyDecision::ForwardToLockWindow => {
                if event.key_down {
                    let manager = MANAGER_WINDOW.load(Ordering::Relaxed) as HWND;
                    if !manager.is_null() {
                        let mut key_lparam = 1_i32 | ((data.scanCode as i32) << 16);
                        if data.flags & LLKHF_EXTENDED != 0 {
                            key_lparam |= 1 << 24;
                        }
                        if data.flags & LLKHF_ALTDOWN != 0 {
                            key_lparam |= 1 << 29;
                        }
                        PostMessageW(
                            manager,
                            wparam as u32,
                            data.vkCode as WPARAM,
                            key_lparam as LPARAM,
                        );
                    }
                }
                1
            }
            KeyDecision::PassThrough => CallNextHookEx(null_mut(), code, wparam, lparam),
        }
    }
}

pub(super) fn configure_win_l_override(enabled: bool) -> Result<()> {
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Policies\System");
    let name = wide("DisableLockWorkstation");
    let current = read_win_l_override(&path, &name);
    if !crate::windows_policy::win_l_registry_update_needed(current, enabled) {
        return Ok(());
    }
    let mut key = null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("não foi possível configurar Win + L: erro {status}");
    }
    let value = u32::from(enabled).to_ne_bytes();
    let status = unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_DWORD, value.as_ptr(), 4) };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        bail!("não foi possível salvar a configuração de Win + L: erro {status}");
    }
    Ok(())
}

fn read_win_l_override(path: &[u16], name: &[u16]) -> Option<u32> {
    let mut key = null_mut();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
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
    unsafe { RegCloseKey(key) };
    (status == ERROR_SUCCESS && value_type == REG_DWORD && size == size_of::<u32>() as u32)
        .then_some(value)
}

fn timer_tick() {
    send_heartbeat_if_due();
    if LOCKED.load(Ordering::SeqCst)
        && let Some(runtime) = RUNTIME.get()
        && let Ok(runtime) = runtime.lock()
    {
        let target = if runtime.prompt.is_null() {
            runtime.overlays.first().copied().unwrap_or(runtime.manager)
        } else {
            runtime.prompt
        };
        unsafe {
            SetWindowPos(target, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            if !is_own_foreground() {
                SetForegroundWindow(target);
            }
            if !runtime.prompt.is_null() {
                InvalidateRect(runtime.prompt, null(), 1);
            } else if runtime.widget.kind == crate::config::WidgetKind::Clock {
                for &overlay in &runtime.overlays {
                    InvalidateRect(overlay, null(), 0);
                }
            }
        }
        if let Some(deadline) = runtime.controller.retry_at()
            && Instant::now() >= deadline
        {
            drop(runtime);
            handle_event(Event::RetryDelayElapsed);
        }
    }
}

fn send_heartbeat_if_due() {
    let Some(runtime) = RUNTIME.get() else { return };
    let should_send = match runtime.lock() {
        Ok(mut runtime) if runtime.last_heartbeat.elapsed().as_secs() >= 2 => {
            runtime.last_heartbeat = Instant::now();
            true
        }
        _ => false,
    };
    if should_send {
        let request = crate::protocol::ClientRequest::Heartbeat {
            locked: LOCKED.load(Ordering::SeqCst),
        };
        let _ = ipc::send_current_session(&request);
    }
}

fn start_agent_pipe(window: HWND) -> Result<()> {
    let names = crate::protocol::PipeNames::for_session(current_session_id()?);
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let window_value = window as isize;
    std::thread::spawn(move || {
        let _ = ipc::serve(
            names.agent,
            current_session_id().unwrap_or(u32::MAX),
            stop,
            None,
            move |request, _client_process| match request {
                crate::protocol::ClientRequest::Lock => {
                    unsafe { PostMessageW(window_value as HWND, WM_LOCK_REQUEST, 0, 0) };
                    crate::protocol::ServiceResponse::Ok
                }
                crate::protocol::ClientRequest::Status => {
                    crate::protocol::ServiceResponse::Status {
                        enabled: true,
                        agent_running: true,
                        locked: LOCKED.load(Ordering::SeqCst),
                        last_error: None,
                    }
                }
                _ => crate::protocol::ServiceResponse::Error {
                    message: "comando não aceito pelo agente".into(),
                },
            },
        );
    });
    Ok(())
}

fn is_own_foreground() -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut process_id = 0;
        GetWindowThreadProcessId(foreground, &mut process_id);
        process_id == GetCurrentProcessId()
    }
}

fn register_hotkey(window: HWND) -> Result<()> {
    let (modifiers, key, label) =
        match ipc::send_current_session(&crate::protocol::ClientRequest::Settings) {
            Ok(crate::protocol::ServiceResponse::Settings { hotkey, .. }) => {
                let mut modifiers = MOD_NOREPEAT;
                if hotkey.control {
                    modifiers |= MOD_CONTROL;
                }
                if hotkey.alt {
                    modifiers |= MOD_ALT;
                }
                if hotkey.shift {
                    modifiers |= MOD_SHIFT;
                }
                let key = hotkey
                    .key
                    .chars()
                    .next()
                    .unwrap_or('L')
                    .to_ascii_uppercase() as u32;
                (modifiers, key, hotkey.display_name())
            }
            _ => (
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                b'L' as u32,
                "Ctrl+Shift+L".into(),
            ),
        };
    if unsafe { RegisterHotKey(window, HOTKEY_ID, modifiers, key) } == 0 {
        bail!("o atalho {label} já está em uso");
    }
    Ok(())
}

fn add_tray_icon(window: HWND) -> Result<()> {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    let instance = unsafe { GetModuleHandleW(null()) };
    let embedded_icon = unsafe { LoadIconW(instance, std::ptr::without_provenance(1)) };
    data.hIcon = if embedded_icon.is_null() {
        unsafe { LoadIconW(null_mut(), IDI_APPLICATION) }
    } else {
        embedded_icon
    };
    copy_wide(&mut data.szTip, DISPLAY_NAME);
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
        bail!("não foi possível criar o ícone da bandeja");
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
    Ok(())
}

fn delete_tray_icon(window: HWND) {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = 1;
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

fn show_tray_menu(window: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let lock = wide("Bloquear agora");
        let settings = wide("Configurações");
        let status = wide("Estado");
        AppendMenuW(menu, MF_STRING, MENU_LOCK, lock.as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_SETTINGS, settings.as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_STATUS, status.as_ptr());
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        );
        DestroyMenu(menu);
        if command != 0 {
            PostMessageW(window, WM_COMMAND, command as WPARAM, 0);
        }
    }
}

fn handle_menu(command: usize) {
    match command {
        MENU_LOCK => {
            let _ = request_lock();
        }
        MENU_SETTINGS => {
            let _ = spawn_cli("settings");
        }
        MENU_STATUS => {
            let _ = spawn_cli("status");
        }
        _ => {}
    }
}

fn spawn_cli(command: &str) -> Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg(command)
        .spawn()
        .context("não foi possível abrir o comando")?;
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    let encoded = value.encode_utf16().take(N.saturating_sub(1));
    for (slot, character) in target.iter_mut().zip(encoded) {
        *slot = character;
    }
}
