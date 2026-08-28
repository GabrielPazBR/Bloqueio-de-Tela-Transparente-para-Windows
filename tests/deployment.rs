use bloqueio_transparente::deployment::{
    FirstRunAction, MaintenanceAction, SetupInputError, ShortcutOptions, desktop_entry,
    first_run_action, legacy_start_menu_entry, maintenance_actions, settings_launch,
    shortcut_entries, start_menu_entry, validate_setup_password,
};
use std::path::Path;

#[test]
fn first_double_click_requests_setup_and_later_opens_maintenance() {
    assert_eq!(
        first_run_action(false, false),
        FirstRunAction::RequestElevatedSetup
    );
    assert_eq!(
        first_run_action(true, true),
        FirstRunAction::OpenMaintenance
    );
}

#[test]
fn maintenance_offers_settings_repair_and_uninstall() {
    assert_eq!(
        maintenance_actions(Some("0.1.0"), "0.1.0"),
        vec![
            MaintenanceAction::OpenSettings,
            MaintenanceAction::Repair,
            MaintenanceAction::Uninstall,
        ]
    );
}

#[test]
fn maintenance_offers_update_only_for_a_different_or_unknown_installed_version() {
    assert_eq!(
        maintenance_actions(Some("0.1.0"), "0.2.0"),
        vec![
            MaintenanceAction::OpenSettings,
            MaintenanceAction::Update,
            MaintenanceAction::Repair,
            MaintenanceAction::Uninstall,
        ]
    );
    assert!(maintenance_actions(None, "0.2.0").contains(&MaintenanceAction::Update));
}

#[test]
fn partial_installation_never_asks_for_a_new_password() {
    assert_eq!(
        first_run_action(true, false),
        FirstRunAction::OpenMaintenance
    );
    assert_eq!(
        first_run_action(false, true),
        FirstRunAction::OpenMaintenance
    );
}

#[test]
fn installation_exposes_a_start_menu_entry() {
    assert_eq!(
        start_menu_entry(Path::new(r"C:\ProgramData")),
        Path::new(
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Bloqueio Transparente.lnk"
        )
    );
    assert_eq!(
        legacy_start_menu_entry(Path::new(r"C:\ProgramData")),
        Path::new(
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Bloqueio Transparente.exe"
        )
    );
}

#[test]
fn setup_can_create_start_menu_and_desktop_shortcuts_independently() {
    let program_data = Path::new(r"C:\ProgramData");
    let user_profile = Path::new(r"C:\Users\Gabriel");
    assert_eq!(
        desktop_entry(user_profile),
        Path::new(r"C:\Users\Gabriel\Desktop\Bloqueio Transparente.lnk")
    );
    assert_eq!(
        ShortcutOptions::default(),
        ShortcutOptions {
            start_menu: true,
            desktop: false,
        }
    );
    assert_eq!(
        shortcut_entries(
            program_data,
            user_profile,
            ShortcutOptions {
                start_menu: true,
                desktop: true,
            },
        ),
        vec![start_menu_entry(program_data), desktop_entry(user_profile)]
    );
    assert!(
        shortcut_entries(
            program_data,
            user_profile,
            ShortcutOptions {
                start_menu: false,
                desktop: false,
            }
        )
        .is_empty()
    );
}

#[test]
fn successful_installation_launches_the_installed_settings_command() {
    let target = Path::new(r"C:\Program Files\Bloqueio Transparente\BloqueioTransparente.exe");
    let (executable, argument) = settings_launch(target);
    assert_eq!(executable, target);
    assert_eq!(argument, "settings");
}

#[test]
fn setup_accepts_an_empty_password_but_requires_confirmation() {
    assert_eq!(validate_setup_password("", ""), Ok(()));
    assert_eq!(
        validate_setup_password("senha", "outra"),
        Err(SetupInputError::PasswordConfirmationMismatch)
    );
}
