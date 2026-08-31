use super::{DISPLAY_NAME, ipc};
use crate::config::{Hotkey, IDLE_TIMEOUT_OPTIONS_MINUTES};
use crate::protocol::{ClientRequest, ServiceResponse};
use crate::settings_ui::{
    ProtectionStatus, SettingsModel, WidgetSizePreset, WindowsHelloButtonAction,
    windows_hello_button_action,
};
use anyhow::{Context, Result, anyhow};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FileOpenDialog, IFileOpenDialog,
    SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, w};
use windows_sys::Win32::System::Console::GetConsoleWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, ShowWindow,
};
use zeroize::Zeroize;

const BACKGROUND: Color32 = Color32::from_rgb(20, 22, 25);
const SIDEBAR: Color32 = Color32::from_rgb(25, 27, 30);
const SURFACE: Color32 = Color32::from_rgb(34, 37, 41);
const SURFACE_HOVER: Color32 = Color32::from_rgb(43, 47, 52);
const BORDER: Color32 = Color32::from_rgb(58, 63, 70);
const TEXT: Color32 = Color32::from_rgb(242, 244, 247);
const MUTED: Color32 = Color32::from_rgb(176, 183, 193);
const PRIMARY: Color32 = Color32::from_rgb(105, 174, 235);
const PRIMARY_DARK: Color32 = Color32::from_rgb(34, 102, 164);
const SUCCESS: Color32 = Color32::from_rgb(90, 201, 134);
const ERROR: Color32 = Color32::from_rgb(244, 113, 116);
const CONTROL_BACKGROUND: Color32 = Color32::from_rgb(15, 17, 20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Lock,
    Password,
    Shortcut,
    Appearance,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirmation {
    Protection(bool),
}

struct Notice {
    text: String,
    error: bool,
    created: Instant,
}

pub fn run() -> Result<()> {
    hide_console();
    let apartment_initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
    let model = match load_model() {
        Ok(model) => model,
        Err(error) => {
            show_settings_error(&error.to_string());
            if apartment_initialized {
                unsafe { CoUninitialize() };
            }
            return Err(error);
        }
    };
    let options = eframe::NativeOptions {
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{DISPLAY_NAME} - Configurações"))
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([860.0, 620.0])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    let result = eframe::run_native(
        DISPLAY_NAME,
        options,
        Box::new(move |context| {
            configure_style(&context.egui_ctx);
            Ok(Box::new(SettingsApp::new(model, &context.egui_ctx)))
        }),
    )
    .map_err(|error| anyhow!(error.to_string()));
    if apartment_initialized {
        unsafe { CoUninitialize() };
    }
    result
}

fn show_settings_error(details: &str) {
    let message = format!(
        "Não foi possível abrir as configurações.\n\n{details}\n\nReinicie o computador e tente novamente."
    );
    let message = message
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let title = format!("{DISPLAY_NAME} - Erro")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub(super) fn hide_console() {
    unsafe {
        let console = GetConsoleWindow();
        if !console.is_null() {
            ShowWindow(console, SW_HIDE);
        }
    }
}

fn load_model() -> Result<SettingsModel> {
    let settings = ipc::send_current_session(&ClientRequest::Settings)
        .context("não foi possível carregar as configurações")?;
    let ServiceResponse::Settings {
        enabled,
        windows_hello_enabled,
        win_l_enabled,
        idle_timeout_minutes,
        dimming_percentage,
        unlock_message,
        hide_taskbar_on_lock,
        widget,
        unlock_logo_path,
        hotkey,
    } = settings
    else {
        return Err(response_error(settings));
    };
    let status = ipc::send_current_session(&ClientRequest::Status)
        .context("não foi possível consultar o estado")?;
    let ServiceResponse::Status {
        agent_running,
        locked,
        last_error,
        ..
    } = status
    else {
        return Err(response_error(status));
    };
    Ok(SettingsModel::new(
        enabled,
        windows_hello_enabled,
        win_l_enabled,
        idle_timeout_minutes,
        dimming_percentage,
        unlock_message,
        hide_taskbar_on_lock,
        widget,
        unlock_logo_path,
        hotkey,
        ProtectionStatus {
            agent_running,
            locked,
            last_error,
        },
    ))
}

fn response_error(response: ServiceResponse) -> anyhow::Error {
    match response {
        ServiceResponse::Error { message } => anyhow!(message),
        other => anyhow!("resposta inesperada do serviço: {other:?}"),
    }
}

fn idle_timeout_label(minutes: u16) -> String {
    match minutes {
        0 => "Desativado".into(),
        1 => "Após 1 minuto".into(),
        value => format!("Após {value} minutos"),
    }
}

struct SettingsApp {
    model: SettingsModel,
    page: Page,
    confirmation: Option<Confirmation>,
    confirmation_password: String,
    current_password: String,
    hello_password: String,
    hello_activation_receiver: Option<Receiver<super::windows_hello::VerificationOutcome>>,
    hello_activation_cancellation: Option<super::windows_hello::VerificationCancellation>,
    new_password: String,
    password_confirmation: String,
    shortcut_password: String,
    win_l_password: String,
    shortcut: Hotkey,
    saved_dimming_percentage: u8,
    lock_texture: egui::TextureHandle,
    unlock_logo_preview_path: Option<String>,
    unlock_logo_preview: Option<egui::TextureHandle>,
    unlock_logo_preview_error: Option<String>,
    notice: Option<Notice>,
}

impl SettingsApp {
    fn new(model: SettingsModel, context: &egui::Context) -> Self {
        let shortcut = model.hotkey.clone();
        let saved_dimming_percentage = model.dimming_percentage;
        let icon = app_icon();
        let lock_texture = context.load_texture(
            "bloqueio-transparente-lock",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            model,
            page: Page::Lock,
            confirmation: None,
            confirmation_password: String::new(),
            current_password: String::new(),
            hello_password: String::new(),
            hello_activation_receiver: None,
            hello_activation_cancellation: None,
            new_password: String::new(),
            password_confirmation: String::new(),
            shortcut_password: String::new(),
            win_l_password: String::new(),
            shortcut,
            saved_dimming_percentage,
            lock_texture,
            unlock_logo_preview_path: None,
            unlock_logo_preview: None,
            unlock_logo_preview_error: None,
            notice: None,
        };
        app.sync_unlock_logo_preview(context);
        app
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            created: Instant::now(),
        });
    }

    fn send(&mut self, request: ClientRequest, success: &str) -> bool {
        match ipc::send_current_session(&request) {
            Ok(ServiceResponse::Ok) => {
                self.notify(success, false);
                true
            }
            Ok(ServiceResponse::Error { message }) => {
                self.notify(message, true);
                false
            }
            Ok(other) => {
                self.notify(format!("Resposta inesperada: {other:?}"), true);
                false
            }
            Err(error) => {
                self.notify(error.to_string(), true);
                false
            }
        }
    }

    fn start_windows_hello_activation(&mut self, context: &egui::Context) {
        let owner = unsafe { GetForegroundWindow() };
        if owner.is_null() {
            self.notify(
                "Não foi possível identificar a janela de configurações.",
                true,
            );
            return;
        }

        let (sender, receiver) = mpsc::channel();
        let context = context.clone();
        let cancellation = super::windows_hello::verify_activation_for_window_async(
            owner as isize,
            "Confirme sua identidade para ativar o Windows Hello".to_owned(),
            move |outcome| {
                let _ = sender.send(outcome);
                context.request_repaint();
            },
        );
        self.hello_activation_receiver = Some(receiver);
        self.hello_activation_cancellation = Some(cancellation);
    }

    fn set_windows_hello_enabled(&mut self, enabled: bool) {
        let request = SettingsModel::set_windows_hello_request(&self.hello_password, enabled);
        let success = if enabled {
            "Windows Hello ativado como único desbloqueio."
        } else {
            "Windows Hello desativado."
        };
        if self.send(request, success) {
            self.model.windows_hello_enabled = enabled;
            clear(&mut self.hello_password);
        }
    }

    fn poll_windows_hello_activation(&mut self) {
        let outcome = match self
            .hello_activation_receiver
            .as_ref()
            .map(Receiver::try_recv)
        {
            Some(Ok(outcome)) => outcome,
            Some(Err(TryRecvError::Disconnected)) => {
                super::windows_hello::VerificationOutcome::Rejected(
                    "A verificação do Windows Hello foi interrompida.".to_owned(),
                )
            }
            Some(Err(TryRecvError::Empty)) | None => return,
        };
        self.hello_activation_receiver = None;
        self.hello_activation_cancellation = None;
        match outcome {
            super::windows_hello::VerificationOutcome::Verified => {
                self.set_windows_hello_enabled(true);
            }
            super::windows_hello::VerificationOutcome::Canceled => {
                self.notify("Verificação cancelada.", true);
            }
            super::windows_hello::VerificationOutcome::Rejected(message) => {
                self.notify(message, true);
            }
        }
    }

    fn refresh(&mut self) {
        match load_model() {
            Ok(model) => {
                self.shortcut = model.hotkey.clone();
                self.saved_dimming_percentage = model.dimming_percentage;
                self.model = model;
                self.notify("Estado atualizado.", false);
            }
            Err(error) => self.notify(error.to_string(), true),
        }
    }

    fn sync_unlock_logo_preview(&mut self, context: &egui::Context) {
        let path = self
            .model
            .unlock_logo_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned);
        if self.unlock_logo_preview_path == path {
            return;
        }

        self.unlock_logo_preview_path = path.clone();
        self.unlock_logo_preview = None;
        self.unlock_logo_preview_error = None;
        let Some(path) = path else { return };
        match load_preview_texture(context, &path) {
            Ok(texture) => self.unlock_logo_preview = Some(texture),
            Err(error) => self.unlock_logo_preview_error = Some(error.to_string()),
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(SIDEBAR)
            .inner_margin(egui::Margin::symmetric(16, 20))
            .show(ui, |ui| {
                ui.set_width(216.0);
                ui.set_min_height(ui.available_height());
                ui.vertical(|ui| {
                    ui.image((self.lock_texture.id(), Vec2::splat(64.0)));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Bloqueio").size(22.0).strong().color(TEXT));
                    ui.label(
                        RichText::new("Transparente")
                            .size(22.0)
                            .strong()
                            .color(PRIMARY),
                    );
                    ui.add_space(28.0);
                    self.nav_item(ui, Page::Lock, "Bloqueio");
                    self.nav_item(ui, Page::Password, "Senha");
                    self.nav_item(ui, Page::Shortcut, "Atalho");
                    self.nav_item(ui, Page::Appearance, "Aparência");
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        self.nav_item(ui, Page::About, "Sobre");
                    });
                });
            });
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, page: Page, label: &str) {
        let selected = self.page == page;
        let button = egui::Button::new(RichText::new(label).size(15.0).color(if selected {
            TEXT
        } else {
            MUTED
        }))
        .fill(if selected {
            SURFACE_HOVER
        } else {
            Color32::TRANSPARENT
        })
        .stroke(if selected {
            Stroke::new(1.0, BORDER)
        } else {
            Stroke::NONE
        })
        .corner_radius(7.0)
        .min_size(Vec2::new(184.0, 44.0));
        if ui.add(button).clicked() {
            self.page = page;
        }
        ui.add_space(4.0);
    }

    fn lock_page(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Bloqueio",
            "Gerencie a proteção e consulte o estado atual.",
        );
        self.status_card(ui);
        ui.add_space(16.0);
        self.protection_card(ui);
        ui.add_space(12.0);
        self.inactivity_card(ui);
        ui.add_space(12.0);
        self.dimming_card(ui);
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Testar o bloqueio")
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("Bloqueia agora mantendo os monitores visíveis.")
                            .color(MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Bloquear agora").clicked() {
                        match ipc::send_current_session(&ClientRequest::Lock) {
                            Ok(ServiceResponse::Ok) => self.notify("Bloqueio solicitado.", false),
                            Ok(other) => self.notify(response_error(other).to_string(), true),
                            Err(error) => self.notify(error.to_string(), true),
                        }
                    }
                });
            });
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let color = if self.model.enabled && self.model.status.agent_running {
                    SUCCESS
                } else {
                    ERROR
                };
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 6.0, color);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(self.model.status_label())
                            .size(18.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(RichText::new(self.model.screen_label()).color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, "Atualizar").clicked() {
                        self.refresh();
                    }
                });
            });
            if let Some(error) = &self.model.status.last_error {
                ui.add_space(10.0);
                ui.label(RichText::new(error).color(ERROR));
            }
        });
    }

    fn protection_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Proteção transparente")
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(
                            "Inicia o agente com o Windows e restaura o bloqueio se ele falhar.",
                        )
                        .color(MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut value = self.model.enabled;
                    let label = if value { "Ativada" } else { "Desativada" };
                    if ui.checkbox(&mut value, label).changed() {
                        self.confirmation = Some(Confirmation::Protection(value));
                    }
                });
            });
        });
    }

    fn dimming_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(
                RichText::new("Escurecimento da tela")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new("Ajusta o brilho visual durante o bloqueio transparente.")
                    .color(MUTED),
            );
            ui.add_space(14.0);

            let mut value = self.model.dimming_percentage;
            let response = ui.add(
                egui::Slider::new(&mut value, 0..=100)
                    .suffix("%")
                    .show_value(true),
            );
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                match SettingsModel::set_dimming_request(value) {
                    Ok(request) => {
                        if self.send(request, "Escurecimento alterado.") {
                            self.model.dimming_percentage = value;
                            self.saved_dimming_percentage = value;
                        } else {
                            self.model.dimming_percentage = self.saved_dimming_percentage;
                        }
                    }
                    Err(error) => {
                        self.model.dimming_percentage = self.saved_dimming_percentage;
                        self.notify(error.to_string(), true);
                    }
                }
            } else if response.changed() {
                self.model.dimming_percentage = value;
            }
        });
    }

    fn inactivity_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(
                RichText::new("Bloqueio por inatividade")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new("Bloqueia a tela após um período sem usar teclado ou mouse.")
                    .color(MUTED),
            );
            ui.add_space(14.0);

            let previous = self.model.idle_timeout_minutes;
            let mut selected = previous;
            egui::ComboBox::from_id_salt("idle-timeout")
                .selected_text(idle_timeout_label(selected))
                .width(220.0)
                .show_ui(ui, |ui| {
                    for minutes in IDLE_TIMEOUT_OPTIONS_MINUTES {
                        ui.selectable_value(&mut selected, minutes, idle_timeout_label(minutes));
                    }
                });

            if selected != previous {
                match SettingsModel::set_idle_timeout_request(selected) {
                    Ok(request) => {
                        if self.send(request, "Bloqueio por inatividade atualizado.") {
                            self.model.idle_timeout_minutes = selected;
                        }
                    }
                    Err(error) => self.notify(error.to_string(), true),
                }
            }
        });
    }

    fn password_page(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Senha",
            "Configure o desbloqueio e a senha que protege as configurações.",
        );
        card(ui, |ui| {
            ui.label(
                RichText::new("Windows Hello")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new(
                    "Quando ativo, somente o Windows Hello desbloqueia a tela. A senha do app continua protegendo as configurações.",
                )
                .color(MUTED),
            );
            ui.add_space(12.0);
            let enabled = self.model.windows_hello_enabled;
            let verification_in_progress = self.hello_activation_receiver.is_some();
            ui.add_enabled_ui(!verification_in_progress, |ui| {
                password_field(ui, "Senha atual do app", &mut self.hello_password);
                ui.add_space(14.0);
                let label = if enabled {
                    "Desativar Windows Hello"
                } else {
                    "Ativar Windows Hello"
                };
                if primary_button(ui, label).clicked() {
                    match windows_hello_button_action(enabled, verification_in_progress) {
                        WindowsHelloButtonAction::StartVerification => {
                            self.start_windows_hello_activation(ui.ctx());
                        }
                        WindowsHelloButtonAction::Disable => {
                            self.set_windows_hello_enabled(false);
                        }
                        WindowsHelloButtonAction::Wait => {}
                    }
                }
            });
            if verification_in_progress {
                ui.label(RichText::new("Aguardando o Windows Hello...").color(MUTED));
            }
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            password_field(ui, "Senha atual", &mut self.current_password);
            ui.add_space(12.0);
            password_field(ui, "Nova senha", &mut self.new_password);
            ui.add_space(12.0);
            password_field(ui, "Confirmar nova senha", &mut self.password_confirmation);
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Salvar senha").clicked() {
                    match SettingsModel::change_password_request(
                        &self.current_password,
                        &self.new_password,
                        &self.password_confirmation,
                    ) {
                        Ok(request) => {
                            if self.send(request, "Senha alterada.") {
                                clear(&mut self.current_password);
                                clear(&mut self.new_password);
                                clear(&mut self.password_confirmation);
                            }
                        }
                        Err(error) => self.notify(error.to_string(), true),
                    }
                }
                ui.label(RichText::new("Pode ficar vazia; limite de 128 caracteres.").color(MUTED));
            });
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            ui.label(
                RichText::new("Mensagem da tela de bloqueio")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.model.unlock_message)
                    .char_limit(crate::config::MAX_UNLOCK_MESSAGE_CHARS)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} / {}",
                        self.model.unlock_message.chars().count(),
                        crate::config::MAX_UNLOCK_MESSAGE_CHARS
                    ))
                    .color(MUTED),
                );
                if primary_button(ui, "Salvar mensagem").clicked() {
                    match SettingsModel::set_unlock_message_request(&self.model.unlock_message) {
                        Ok(request) => {
                            if self.send(request, "Mensagem salva.")
                                && self.model.unlock_message.trim().is_empty()
                            {
                                self.model.unlock_message = crate::config::default_unlock_message();
                            }
                        }
                        Err(error) => self.notify(error.to_string(), true),
                    }
                }
            });
        });
    }

    fn shortcut_page(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Atalho",
            "Escolha a combinação usada para bloquear imediatamente.",
        );
        card(ui, |ui| {
            ui.label(RichText::new("Modificadores").strong().color(TEXT));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.shortcut.control, "Ctrl");
                ui.checkbox(&mut self.shortcut.alt, "Alt");
                ui.checkbox(&mut self.shortcut.shift, "Shift");
            });
            ui.add_space(12.0);
            ui.label(RichText::new("Tecla").strong().color(TEXT));
            ui.add(
                egui::TextEdit::singleline(&mut self.shortcut.key)
                    .char_limit(1)
                    .desired_width(72.0),
            );
            ui.add_space(12.0);
            password_field(ui, "Senha atual", &mut self.shortcut_password);
            ui.add_space(18.0);
            if primary_button(ui, "Salvar atalho").clicked() {
                self.shortcut.key = self.shortcut.key.to_ascii_uppercase();
                match SettingsModel::update_hotkey_request(
                    &self.shortcut_password,
                    self.shortcut.clone(),
                ) {
                    Ok(request) => {
                        if self.send(request, "Atalho salvo. Reinicie o serviço para aplicá-lo.")
                        {
                            self.model.hotkey = self.shortcut.clone();
                            clear(&mut self.shortcut_password);
                        }
                    }
                    Err(error) => self.notify(error.to_string(), true),
                }
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("Atual: {}", self.model.hotkey.display_name())).color(MUTED),
            );
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            ui.label(
                RichText::new("Usar Win + L")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new(
                    "Substitui o bloqueio do Windows pelo bloqueio transparente. O serviço restaura o comportamento normal se o agente parar.",
                )
                .color(MUTED),
            );
            ui.add_space(12.0);
            password_field(ui, "Senha atual", &mut self.win_l_password);
            ui.add_space(14.0);
            let enabled = self.model.win_l_enabled;
            let label = if enabled {
                "Desativar Win + L"
            } else {
                "Ativar Win + L"
            };
            if primary_button(ui, label).clicked() {
                let request = SettingsModel::set_win_l_request(&self.win_l_password, !enabled);
                let success = if enabled {
                    "Win + L voltou a bloquear pelo Windows."
                } else {
                    "Win + L configurado para o bloqueio transparente."
                };
                if self.send(request, success) {
                    self.model.win_l_enabled = !enabled;
                    clear(&mut self.win_l_password);
                }
            }
        });
    }

    fn about_page(&self, ui: &mut egui::Ui) {
        page_title(ui, "Sobre", "Informações do aplicativo.");
        card(ui, |ui| {
            ui.image((self.lock_texture.id(), Vec2::splat(64.0)));
            ui.add_space(12.0);
            ui.label(RichText::new(DISPLAY_NAME).size(20.0).strong().color(TEXT));
            ui.label(RichText::new(format!("Versão {}", env!("CARGO_PKG_VERSION"))).color(MUTED));
            ui.label(RichText::new("Feito por Gabriel Paz").color(MUTED));
            ui.add_space(12.0);
            ui.label(
                RichText::new("Mantém os monitores visíveis enquanto bloqueia teclado e mouse.")
                    .color(TEXT),
            );
            ui.label(RichText::new("Ctrl+Alt+Del continua sob controle do Windows.").color(MUTED));
        });
    }

    fn appearance_page(&mut self, ui: &mut egui::Ui) {
        use crate::config::WidgetKind;
        self.sync_unlock_logo_preview(ui.ctx());
        page_title(
            ui,
            "Aparência",
            "Personalize os elementos exibidos durante o bloqueio.",
        );
        card(ui, |ui| {
            ui.label(
                RichText::new("Widget da tela bloqueada")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(10.0);
            egui::ComboBox::from_label("Conteúdo")
                .selected_text(match self.model.widget.kind {
                    WidgetKind::None => "Nenhum",
                    WidgetKind::Clock => "Data e hora",
                    WidgetKind::Image => "Imagem",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.model.widget.kind, WidgetKind::None, "Nenhum");
                    ui.selectable_value(
                        &mut self.model.widget.kind,
                        WidgetKind::Clock,
                        "Data e hora",
                    );
                    ui.selectable_value(&mut self.model.widget.kind, WidgetKind::Image, "Imagem");
                });
            if self.model.widget.kind == WidgetKind::Image {
                ui.add_space(10.0);
                ui.label("Arquivo da imagem");
                let mut choose_image = false;
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(
                            self.model.widget.image_path.get_or_insert_default(),
                        )
                        .desired_width((ui.available_width() - 132.0).max(180.0)),
                    );
                    choose_image = secondary_button(ui, "Escolher...").clicked();
                });
                if choose_image {
                    match pick_image_file() {
                        Ok(Some(path)) => self.model.widget.image_path = Some(path),
                        Ok(None) => {}
                        Err(error) => self.notify(error.to_string(), true),
                    }
                }
                ui.label(RichText::new("Formatos aceitos: PNG, JPEG e BMP.").color(MUTED));
            }
            if self.model.widget.kind != WidgetKind::None {
                ui.add_space(14.0);
                ui.label(RichText::new("Tamanho").strong().color(TEXT));
                let current_size = WidgetSizePreset::from_widget(&self.model.widget);
                egui::ComboBox::from_id_salt("widget-size")
                    .selected_text(current_size.map_or("Personalizado", WidgetSizePreset::label))
                    .show_ui(ui, |ui| {
                        for preset in WidgetSizePreset::ALL {
                            if ui
                                .selectable_label(current_size == Some(preset), preset.label())
                                .clicked()
                            {
                                preset.apply(&mut self.model.widget);
                            }
                        }
                    });
                ui.add_space(12.0);
                ui.label(RichText::new("Posição").strong().color(TEXT));
                ui.add(
                    egui::Slider::new(&mut self.model.widget.x_percent, 0..=100)
                        .suffix("%")
                        .text("Posição horizontal"),
                );
                ui.add(
                    egui::Slider::new(&mut self.model.widget.y_percent, 0..=100)
                        .suffix("%")
                        .text("Posição vertical"),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Transparência").strong().color(TEXT));
                ui.add(
                    egui::Slider::new(&mut self.model.widget.opacity_percentage, 0..=100)
                        .suffix("%")
                        .text("Transparência do widget"),
                );
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.checkbox(
                &mut self.model.hide_taskbar_on_lock,
                "Ocultar a barra de tarefas durante o bloqueio",
            );
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new("Logo do desbloqueio")
                    .size(16.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(10.0);
            ui.label("Arquivo da logo");
            let mut choose_logo = false;
            let path_changed = ui
                .horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(
                            self.model.unlock_logo_path.get_or_insert_default(),
                        )
                        .desired_width((ui.available_width() - 132.0).max(180.0)),
                    );
                    choose_logo = secondary_button(ui, "Escolher...").clicked();
                    response.changed()
                })
                .inner;
            if choose_logo {
                match pick_image_file() {
                    Ok(Some(path)) => self.model.unlock_logo_path = Some(path),
                    Ok(None) => {}
                    Err(error) => self.notify(error.to_string(), true),
                }
            }
            if path_changed || choose_logo {
                self.sync_unlock_logo_preview(ui.ctx());
            }
            if let Some(texture) = &self.unlock_logo_preview {
                let original = texture.size_vec2();
                let scale = (240.0 / original.x).min(120.0 / original.y).min(1.0);
                ui.add_space(10.0);
                ui.image((texture.id(), original * scale));
            } else if let Some(error) = &self.unlock_logo_preview_error {
                ui.add_space(8.0);
                ui.label(RichText::new(error).color(ERROR));
            }
            ui.label(
                RichText::new("Formatos aceitos: PNG, JPEG e BMP. Deixe vazio para não exibir.")
                    .color(MUTED),
            );
        });
        ui.add_space(16.0);
        if primary_button(ui, "Salvar aparência").clicked() {
            match SettingsModel::set_visual_options_request(
                self.model.hide_taskbar_on_lock,
                self.model.widget.clone(),
                self.model.unlock_logo_path.clone(),
            ) {
                Ok(request) => {
                    self.send(request, "Aparência salva.");
                }
                Err(error) => self.notify(error.to_string(), true),
            }
        }
    }

    fn confirmation_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirmation else {
            return;
        };
        egui::Window::new("Confirmar alteração")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label("Digite a senha atual para confirmar.");
                ui.add_space(10.0);
                password_field(ui, "Senha atual", &mut self.confirmation_password);
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "Confirmar").clicked() {
                        let request = match action {
                            Confirmation::Protection(enabled) => ClientRequest::SetEnabled {
                                current: self.confirmation_password.as_str().into(),
                                enabled,
                            },
                        };
                        if self.send(request, "Configuração alterada.") {
                            match action {
                                Confirmation::Protection(enabled) => self.model.enabled = enabled,
                            }
                            clear(&mut self.confirmation_password);
                            self.confirmation = None;
                        }
                    }
                    if secondary_button(ui, "Cancelar").clicked() {
                        clear(&mut self.confirmation_password);
                        self.confirmation = None;
                    }
                });
            });
    }

    fn notice(&mut self, ctx: &egui::Context) {
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| notice.created.elapsed() > Duration::from_secs(5))
        {
            self.notice = None;
        }
        let Some(notice) = &self.notice else { return };
        egui::Area::new(egui::Id::new("notice"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -24.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(if notice.error {
                        Color32::from_rgb(82, 35, 39)
                    } else {
                        Color32::from_rgb(28, 70, 50)
                    })
                    .stroke(Stroke::new(1.0, if notice.error { ERROR } else { SUCCESS }))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&notice.text).color(TEXT));
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl Drop for SettingsApp {
    fn drop(&mut self) {
        clear(&mut self.confirmation_password);
        clear(&mut self.current_password);
        clear(&mut self.hello_password);
        clear(&mut self.new_password);
        clear(&mut self.password_confirmation);
        clear(&mut self.shortcut_password);
        clear(&mut self.win_l_password);
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_windows_hello_activation();
        let ctx = ui.ctx().clone();
        ui.horizontal_top(|ui| {
            self.sidebar(ui);
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::symmetric(32, 28))
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 1.0).max(560.0));
                    ui.set_min_height(ui.available_height());
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                            Page::Lock => self.lock_page(ui),
                            Page::Password => self.password_page(ui),
                            Page::Shortcut => self.shortcut_page(ui),
                            Page::Appearance => self.appearance_page(ui),
                            Page::About => self.about_page(ui),
                        });
                    });
                });
        });
        self.confirmation_window(&ctx);
        self.notice(&ctx);
    }
}

pub(super) fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(17, 19, 22);
    visuals.widgets.inactive.bg_fill = CONTROL_BACKGROUND;
    visuals.widgets.inactive.weak_bg_fill = CONTROL_BACKGROUND;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, PRIMARY);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_fill = PRIMARY_DARK;
    visuals.widgets.active.weak_bg_fill = PRIMARY_DARK;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, PRIMARY);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.selection.bg_fill = PRIMARY_DARK;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    visuals.slider_trailing_fill = true;
    visuals.hyperlink_color = PRIMARY;
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 9.0);
    style.visuals.window_corner_radius = 10.0.into();
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn pick_image_file() -> Result<Option<String>> {
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .context("não foi possível abrir o seletor de arquivos")?;
    let filters = [COMDLG_FILTERSPEC {
        pszName: w!("Imagens"),
        pszSpec: w!("*.png;*.jpg;*.jpeg;*.bmp"),
    }];
    unsafe {
        dialog.SetFileTypes(&filters)?;
        dialog.SetOptions(FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM)?;
        dialog.SetTitle(w!("Escolher imagem"))?;
        if let Err(error) = dialog.Show(None) {
            if error.code() == HRESULT::from_win32(1223) {
                return Ok(None);
            }
            return Err(error).context("não foi possível escolher o arquivo");
        }
        let path = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let value = path.to_string()?;
        CoTaskMemFree(Some(path.0.cast()));
        Ok(Some(value))
    }
}

fn load_preview_texture(context: &egui::Context, path: &str) -> Result<egui::TextureHandle> {
    let image = image::open(path)
        .with_context(|| format!("Não foi possível carregar a prévia de {path}"))?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Ok(context.load_texture(
        "unlock-logo-preview",
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(28.0).strong().color(TEXT));
    ui.label(RichText::new(subtitle).size(14.0).color(MUTED));
    ui.add_space(24.0);
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

pub(super) fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(PRIMARY_DARK)
            .stroke(Stroke::new(1.0, PRIMARY))
            .corner_radius(6.0)
            .min_size(Vec2::new(128.0, 40.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(TEXT))
            .fill(SURFACE_HOVER)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(6.0)
            .min_size(Vec2::new(104.0, 38.0)),
    )
}

pub(super) fn password_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(RichText::new(label).strong().color(TEXT));
    ui.add(
        egui::TextEdit::singleline(value)
            .password(true)
            .char_limit(128)
            .desired_width(f32::INFINITY),
    );
}

pub(super) fn app_icon() -> egui::IconData {
    let width = 64_u32;
    let height = 64_u32;
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            let dx = x as i32 - 32;
            let dy = y as i32 - 32;
            if dx * dx + dy * dy <= 30 * 30 {
                rgba[index..index + 4].copy_from_slice(&[34, 102, 164, 255]);
            }
            let body = (15..=49).contains(&x) && (29..=52).contains(&y);
            let shackle = (18..=46).contains(&x) && (9..=35).contains(&y) && {
                let outer = ((x as i32 - 32).pow(2) * 100 / 196)
                    + ((y as i32 - 26).pow(2) * 100 / 289)
                    <= 100;
                let inner = ((x as i32 - 32).pow(2) * 100 / 81)
                    + ((y as i32 - 27).pow(2) * 100 / 144)
                    < 100;
                outer && !inner
            };
            if body || shackle {
                rgba[index..index + 4].copy_from_slice(&[242, 244, 247, 255]);
            }
        }
    }
    egui::IconData {
        rgba,
        width,
        height,
    }
}

fn clear(value: &mut String) {
    value.zeroize();
    value.clear();
}
