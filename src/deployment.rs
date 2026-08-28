#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstRunAction {
    RequestElevatedSetup,
    OpenMaintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceAction {
    OpenSettings,
    Update,
    Repair,
    Uninstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutOptions {
    pub start_menu: bool,
    pub desktop: bool,
}

impl Default for ShortcutOptions {
    fn default() -> Self {
        Self {
            start_menu: true,
            desktop: false,
        }
    }
}

pub fn maintenance_actions(
    installed_version: Option<&str>,
    current_version: &str,
) -> Vec<MaintenanceAction> {
    let mut actions = vec![MaintenanceAction::OpenSettings];
    if installed_version != Some(current_version) {
        actions.push(MaintenanceAction::Update);
    }
    actions.extend([MaintenanceAction::Repair, MaintenanceAction::Uninstall]);
    actions
}

pub fn start_menu_entry(program_data: &std::path::Path) -> std::path::PathBuf {
    program_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Bloqueio Transparente.lnk")
}

pub fn legacy_start_menu_entry(program_data: &std::path::Path) -> std::path::PathBuf {
    program_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Bloqueio Transparente.exe")
}

pub fn desktop_entry(user_profile: &std::path::Path) -> std::path::PathBuf {
    user_profile
        .join("Desktop")
        .join("Bloqueio Transparente.lnk")
}

pub fn settings_launch(
    installed_executable: &std::path::Path,
) -> (std::path::PathBuf, &'static str) {
    (installed_executable.to_owned(), "settings")
}

pub fn shortcut_entries(
    program_data: &std::path::Path,
    user_profile: &std::path::Path,
    options: ShortcutOptions,
) -> Vec<std::path::PathBuf> {
    let mut entries = Vec::with_capacity(2);
    if options.start_menu {
        entries.push(start_menu_entry(program_data));
    }
    if options.desktop {
        entries.push(desktop_entry(user_profile));
    }
    entries
}

pub fn first_run_action(executable_exists: bool, config_exists: bool) -> FirstRunAction {
    if executable_exists || config_exists {
        FirstRunAction::OpenMaintenance
    } else {
        FirstRunAction::RequestElevatedSetup
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SetupInputError {
    #[error("as senhas não conferem")]
    PasswordConfirmationMismatch,
}

pub fn validate_setup_password(password: &str, confirmation: &str) -> Result<(), SetupInputError> {
    if password == confirmation {
        Ok(())
    } else {
        Err(SetupInputError::PasswordConfirmationMismatch)
    }
}
