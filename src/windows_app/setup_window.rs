use super::{DISPLAY_NAME, install, settings_window};
use crate::deployment::ShortcutOptions;
use anyhow::{Result, anyhow};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use zeroize::Zeroize;

const BACKGROUND: Color32 = Color32::from_rgb(20, 22, 25);
const SURFACE: Color32 = Color32::from_rgb(34, 37, 41);
const BORDER: Color32 = Color32::from_rgb(58, 63, 70);
const TEXT: Color32 = Color32::from_rgb(242, 244, 247);
const MUTED: Color32 = Color32::from_rgb(176, 183, 193);
const SUCCESS: Color32 = Color32::from_rgb(90, 201, 134);
const ERROR: Color32 = Color32::from_rgb(244, 113, 116);

pub fn run() -> Result<()> {
    settings_window::hide_console();
    let options = eframe::NativeOptions {
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{DISPLAY_NAME} - Configuração inicial"))
            .with_inner_size([620.0, 620.0])
            .with_min_inner_size([540.0, 580.0])
            .with_resizable(false)
            .with_icon(Arc::new(settings_window::app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        DISPLAY_NAME,
        options,
        Box::new(|context| {
            settings_window::configure_style(&context.egui_ctx);
            Ok(Box::new(SetupApp::new(&context.egui_ctx)))
        }),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

struct SetupApp {
    password: String,
    confirmation: String,
    message: Option<(String, bool)>,
    installed: bool,
    installing: bool,
    install_result: Option<Receiver<Result<(), String>>>,
    lock_texture: egui::TextureHandle,
    start_menu_shortcut: bool,
    desktop_shortcut: bool,
}

impl SetupApp {
    fn new(context: &egui::Context) -> Self {
        let icon = settings_window::app_icon();
        let lock_texture = context.load_texture(
            "setup-lock",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        Self {
            password: String::new(),
            confirmation: String::new(),
            message: None,
            installed: false,
            installing: false,
            install_result: None,
            lock_texture,
            start_menu_shortcut: true,
            desktop_shortcut: false,
        }
    }

    fn start_install(&mut self) {
        if let Err(error) =
            crate::deployment::validate_setup_password(&self.password, &self.confirmation)
        {
            self.message = Some((error.to_string(), true));
            return;
        }
        let mut password = zeroize::Zeroizing::new(self.password.clone());
        self.password.zeroize();
        self.password.clear();
        self.confirmation.zeroize();
        self.confirmation.clear();
        let (sender, receiver) = std::sync::mpsc::channel();
        let shortcuts = ShortcutOptions {
            start_menu: self.start_menu_shortcut,
            desktop: self.desktop_shortcut,
        };
        std::thread::spawn(move || {
            let result = install::install_with_password(password.as_str(), shortcuts)
                .map_err(|error| error.to_string());
            password.zeroize();
            let _ = sender.send(result);
        });
        self.installing = true;
        self.install_result = Some(receiver);
        self.message = None;
    }

    fn poll_installation(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.install_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => {
                self.installing = false;
                self.installed = true;
                self.install_result = None;
                match install::open_installed_settings() {
                    Ok(()) => context.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(error) => {
                        self.message = Some((error.to_string(), true));
                    }
                }
            }
            Ok(Err(error)) => {
                self.installing = false;
                self.install_result = None;
                self.message = Some((error, true));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.installing = false;
                self.install_result = None;
                self.message = Some(("A instalação foi interrompida.".into(), true));
            }
        }
    }
}

impl Drop for SetupApp {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
    }
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_installation(ui.ctx());
        if self.installing {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::Frame::new()
            .fill(BACKGROUND)
            .inner_margin(egui::Margin::same(36))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.vertical_centered(|ui| {
                    ui.image((self.lock_texture.id(), Vec2::splat(64.0)));
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Configuração inicial")
                            .size(28.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(
                            "O aplicativo será iniciado com o Windows e ficará disponível na bandeja.",
                        )
                        .color(MUTED),
                    );
                    ui.add_space(24.0);
                });

                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::same(22))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if self.installed {
                            ui.label(RichText::new("Pronto").size(20.0).strong().color(SUCCESS));
                            ui.add_space(8.0);
                            ui.label(RichText::new("Atalho padrão: Ctrl+Shift+L").color(TEXT));
                            ui.add_space(20.0);
                            if settings_window::primary_button(ui, "Concluir").clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        } else {
                            settings_window::password_field(ui, "Defina a senha", &mut self.password);
                            ui.add_space(14.0);
                            settings_window::password_field(
                                ui,
                                "Repita a senha",
                                &mut self.confirmation,
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("A senha pode ficar vazia; limite de 128 caracteres.")
                                    .color(MUTED),
                            );
                            ui.add_space(18.0);
                            ui.label(RichText::new("Adicionar atalhos").strong().color(TEXT));
                            ui.add_space(6.0);
                            ui.add_enabled_ui(!self.installing, |ui| {
                                ui.checkbox(&mut self.start_menu_shortcut, "Menu Iniciar");
                                ui.checkbox(&mut self.desktop_shortcut, "Área de trabalho");
                            });
                            ui.add_space(20.0);
                            if self.installing {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Instalando...").color(TEXT));
                                });
                            } else if settings_window::primary_button(ui, "Instalar e iniciar")
                                .clicked()
                            {
                                self.start_install();
                            }
                        }

                        if let Some((message, error)) = &self.message {
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new(message)
                                    .color(if *error { ERROR } else { SUCCESS }),
                            );
                        }
                    });
                ui.add_space(14.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Feito por Gabriel Paz").color(MUTED));
                });
            });
    }
}
