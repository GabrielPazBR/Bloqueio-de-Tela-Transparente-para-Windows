use bloqueio_transparente::config::{Hotkey, WidgetConfig, WidgetKind};
use bloqueio_transparente::protocol::ClientRequest;
use bloqueio_transparente::settings_ui::{
    ProtectionStatus, SettingsInputError, SettingsModel, WidgetSizePreset,
    WindowsHelloButtonAction, windows_hello_button_action,
};

#[test]
fn settings_window_represents_service_configuration_and_status() {
    let model = SettingsModel::new(
        true,
        true,
        false,
        15,
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
    assert!(model.windows_hello_enabled);
    assert_eq!(model.idle_timeout_minutes, 15);
    assert_eq!(model.dimming_percentage, 35);
    assert_eq!(model.unlock_message, "Digite a senha");
    assert_eq!(model.hotkey.display_name(), "Ctrl+Shift+L");
    assert_eq!(model.status_label(), "Proteção ativa");
    assert_eq!(model.screen_label(), "Tela liberada");
}

#[test]
fn inactivity_timeout_accepts_only_the_options_shown_by_the_app() {
    assert_eq!(
        SettingsModel::set_idle_timeout_request(15).unwrap(),
        ClientRequest::SetIdleTimeout { minutes: 15 }
    );
    assert_eq!(
        SettingsModel::set_idle_timeout_request(7).unwrap_err(),
        SettingsInputError::InvalidIdleTimeout
    );
}

#[test]
fn windows_hello_toggle_is_authenticated_with_the_app_password() {
    let request = SettingsModel::set_windows_hello_request("senha", true);
    let ClientRequest::SetWindowsHelloEnabled { current, enabled } = request else {
        panic!("comando inesperado");
    };
    assert_eq!(current.expose(), "senha");
    assert!(enabled);
}

#[test]
fn windows_hello_button_starts_one_non_blocking_verification_at_a_time() {
    assert_eq!(
        windows_hello_button_action(false, false),
        WindowsHelloButtonAction::StartVerification
    );
    assert_eq!(
        windows_hello_button_action(false, true),
        WindowsHelloButtonAction::Wait
    );
    assert_eq!(
        windows_hello_button_action(true, false),
        WindowsHelloButtonAction::Disable
    );
}

#[test]
fn win_l_replacement_toggle_is_authenticated_with_the_app_password() {
    let request = SettingsModel::set_win_l_request("senha", true);
    let ClientRequest::SetWinLEnabled { current, enabled } = request else {
        panic!("comando inesperado");
    };
    assert_eq!(current.expose(), "senha");
    assert!(enabled);
}

#[test]
fn widget_size_presets_use_friendly_stable_dimensions() {
    let mut widget = WidgetConfig::default();

    WidgetSizePreset::ExtraSmall.apply(&mut widget);
    assert_eq!((widget.width, widget.height), (160, 60));
    assert_eq!(
        WidgetSizePreset::from_widget(&widget),
        Some(WidgetSizePreset::ExtraSmall)
    );

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

    WidgetSizePreset::ExtraLarge.apply(&mut widget);
    assert_eq!((widget.width, widget.height), (800, 260));
}

#[test]
fn appearance_form_builds_a_visual_options_command() {
    let widget = WidgetConfig {
        kind: WidgetKind::Clock,
        width: 480,
        height: 140,
        x_percent: 50,
        y_percent: 6,
        opacity_percentage: 72,
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
fn appearance_form_rejects_widget_opacity_above_one_hundred() {
    let widget = WidgetConfig {
        opacity_percentage: 101,
        ..WidgetConfig::default()
    };

    assert_eq!(
        SettingsModel::set_visual_options_request(false, widget, None).unwrap_err(),
        SettingsInputError::InvalidWidget
    );
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
