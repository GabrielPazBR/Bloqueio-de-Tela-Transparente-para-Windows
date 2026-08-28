use super::{DISPLAY_NAME, install, settings_window};
use anyhow::{Result, anyhow};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::sync::Arc;

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
            .with_title(format!("{DISPLAY_NAME} - Manutenção"))
            .with_inner_size([620.0, 520.0])
            .with_min_inner_size([540.0, 470.0])
            .with_resizable(false)
            .with_icon(Arc::new(settings_window::app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        DISPLAY_NAME,
        options,
        Box::new(|context| {
            settings_window::configure_style(&context.egui_ctx);
            Ok(Box::new(MaintenanceApp::new(&context.egui_ctx)))
        }),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

struct MaintenanceApp {
    lock_texture: egui::TextureHandle,
    message: Option<(String, bool)>,
    installed_version: Option<String>,
    update_available: bool,
}

impl MaintenanceApp {
    fn new(context: &egui::Context) -> Self {
        let icon = settings_window::app_icon();
        let lock_texture = context.load_texture(
            "maintenance-lock",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let installed_version = install::installed_version();
        let update_available = crate::deployment::maintenance_actions(
            installed_version.as_deref(),
            env!("CARGO_PKG_VERSION"),
        )
        .contains(&crate::deployment::MaintenanceAction::Update);
        Self {
            lock_texture,
            message: None,
            installed_version,
            update_available,
        }
    }

    fn run_action(&mut self, action: impl FnOnce() -> Result<()>, launched_message: Option<&str>) {
        match action() {
            Ok(()) => {
                self.message = launched_message.map(|message| (message.to_owned(), false));
            }
            Err(error) => {
                self.message = Some((error.to_string(), true));
            }
        }
    }
}

impl eframe::App for MaintenanceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::new()
            .fill(BACKGROUND)
            .inner_margin(egui::Margin::same(36))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.vertical_centered(|ui| {
                    ui.image((self.lock_texture.id(), Vec2::splat(64.0)));
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Bloqueio Transparente")
                            .size(28.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(RichText::new("O aplicativo já está instalado.").color(MUTED));
                    ui.label(
                        RichText::new(format!(
                            "Instalada: {}   Disponível: {}",
                            self.installed_version
                                .as_deref()
                                .unwrap_or("versão anterior"),
                            env!("CARGO_PKG_VERSION")
                        ))
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
                        if settings_window::primary_button(ui, "Abrir configurações").clicked() {
                            self.run_action(install::request_settings, None);
                        }
                        if self.update_available {
                            ui.add_space(12.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 42.0],
                                    egui::Button::new("Atualizar versão"),
                                )
                                .clicked()
                            {
                                self.run_action(
                                    install::request_elevated_update,
                                    Some("Atualização iniciada. O resultado será exibido ao concluir."),
                                );
                            }
                        }
                        ui.add_space(12.0);
                        if ui
                            .add_sized(
                                [ui.available_width(), 42.0],
                                egui::Button::new("Restaurar instalação"),
                            )
                            .clicked()
                        {
                            self.run_action(
                                install::request_elevated_repair,
                                Some("Restauração iniciada. O resultado será exibido ao concluir."),
                            );
                        }
                        ui.add_space(12.0);
                        if ui
                            .add_sized(
                                [ui.available_width(), 42.0],
                                egui::Button::new("Desinstalar"),
                            )
                            .clicked()
                        {
                            self.run_action(
                                install::request_elevated_uninstall,
                                Some("Desinstalação iniciada. O resultado será exibido ao concluir."),
                            );
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
