use bloqueio_transparente::config::{Hotkey, WidgetConfig, WidgetKind};
use bloqueio_transparente::protocol::ClientRequest;
use bloqueio_transparente::settings_ui::{
    ProtectionStatus, SettingsInputError, SettingsModel, WidgetSizePreset,
};

#[test]
fn settings_window_represents_service_configuration_and_status() {
    let model = SettingsModel::new(
        true,
        35,
        "Digite a senha".into(),
        false,
        WidgetConfig::default(),
        None,
        Hotkey::default(),
        ProtectionStatus {
            agent_running: true,
            locked: false,
            last_error: None,
        },
    );

    assert!(model.enabled);
    assert_eq!(model.dimming_percentage, 35);
    assert_eq!(model.unlock_message, "Digite a senha");
    assert_eq!(model.hotkey.display_name(), "Ctrl+Shift+L");
    assert_eq!(model.status_label(), "Proteção ativa");
    assert_eq!(model.screen_label(), "Tela liberada");
}

#[test]
fn widget_size_presets_use_friendly_stable_dimensions() {
    let mut widget = WidgetConfig::default();

    WidgetSizePreset::Small.apply(&mut widget);
    assert_eq!((widget.width, widget.height), (240, 80));
    assert_eq!(
        WidgetSizePreset::from_widget(&widget),
        Some(WidgetSizePreset::Small)
    );

    WidgetSizePreset::Medium.apply(&mut widget);
    assert_eq!((widget.width, widget.height), (400, 120));

    WidgetSizePreset::Large.apply(&mut widget);
    assert_eq!((widget.width, widget.height), (640, 200));
}

#[test]
fn appearance_form_builds_a_visual_options_command() {
    let widget = WidgetConfig {
        kind: WidgetKind::Clock,
        width: 480,
        height: 140,
        x_percent: 50,
        y_percent: 6,
        image_path: None,
    };
    let request = SettingsModel::set_visual_options_request(
        true,
        widget.clone(),
        Some("C:\\logo.png".into()),
    )
    .unwrap();
    let ClientRequest::SetVisualOptions {
        hide_taskbar_on_lock,
        widget: actual,
        unlock_logo_path,
    } = request
    else {
        panic!("comando inesperado")
    };
    assert!(hide_taskbar_on_lock);
    assert_eq!(actual, widget);
    assert_eq!(unlock_logo_path.as_deref(), Some("C:\\logo.png"));
}

#[test]
fn unlock_message_builds_a_validated_service_command() {
    let request = SettingsModel::set_unlock_message_request("Acesso restrito").unwrap();
    assert_eq!(
        request,
        ClientRequest::SetUnlockMessage {
            message: "Acesso restrito".into()
        }
    );

    assert_eq!(
        SettingsModel::set_unlock_message_request("linha 1\nlinha 2").unwrap_err(),
        SettingsInputError::InvalidUnlockMessage
    );
}

#[test]
fn dimming_slider_builds_a_bounded_service_command() {
    let request = SettingsModel::set_dimming_request(60).unwrap();
    assert_eq!(request, ClientRequest::SetDimming { percent: 60 });

    assert_eq!(
        SettingsModel::set_dimming_request(101).unwrap_err(),
        SettingsInputError::InvalidDimmingPercentage
    );
}

#[test]
fn password_form_requires_matching_confirmation_and_builds_the_service_command() {
    assert_eq!(
        SettingsModel::change_password_request("atual", "nova", "outra").unwrap_err(),
        SettingsInputError::PasswordConfirmationMismatch
    );

    let request = SettingsModel::change_password_request("atual", "nova", "nova").unwrap();
    let ClientRequest::ChangePassword { current, new } = request else {
        panic!("comando inesperado");
    };
    assert_eq!(current.expose(), "atual");
    assert_eq!(new.expose(), "nova");
}

#[test]
fn shortcut_form_requires_two_modifiers_and_builds_the_service_command() {
    let weak = Hotkey {
        control: true,
        alt: false,
        shift: false,
        key: "K".into(),
    };
    assert_eq!(
        SettingsModel::update_hotkey_request("senha", weak).unwrap_err(),
        SettingsInputError::NotEnoughModifiers
    );

    let request = SettingsModel::update_hotkey_request("senha", Hotkey::default()).unwrap();
    let ClientRequest::UpdateHotkey { current, hotkey } = request else {
        panic!("comando inesperado");
    };
    assert_eq!(current.expose(), "senha");
    assert_eq!(hotkey.display_name(), "Ctrl+Shift+L");
}
