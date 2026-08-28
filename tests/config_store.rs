use bloqueio_transparente::config::{AppConfig, ConfigError, ConfigStore, Hotkey};
use std::fs;

#[test]
fn existing_configuration_defaults_win_l_override_to_disabled() {
    let mut value = serde_json::to_value(AppConfig::for_test()).unwrap();
    value.as_object_mut().unwrap().remove("win_l_enabled");
    let config: AppConfig = serde_json::from_value(value).unwrap();

    assert!(!config.win_l_enabled);
}

#[test]
fn suspended_integrations_are_disabled_in_existing_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(directory.path().join("config.json"));
    store.initialize("senha", Hotkey::default()).unwrap();
    let mut config = store.load().unwrap();
    config.windows_hello_enabled = true;
    config.win_l_enabled = true;
    store.save(&config).unwrap();

    assert!(store.suspend_unstable_features().unwrap());
    let config = store.load().unwrap();
    assert!(!config.windows_hello_enabled);
    assert!(!config.win_l_enabled);
    assert!(!store.suspend_unstable_features().unwrap());
}

#[test]
fn existing_configuration_defaults_visual_customization_to_disabled() {
    let mut value = serde_json::to_value(AppConfig::for_test()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("hide_taskbar_on_lock");
    object.remove("widget");
    object.remove("unlock_logo_path");
    let config: AppConfig = serde_json::from_value(value).unwrap();

    assert!(!config.hide_taskbar_on_lock);
    assert_eq!(
        config.widget.kind,
        bloqueio_transparente::config::WidgetKind::None
    );
    assert!(config.unlock_logo_path.is_none());
}

#[test]
fn new_configuration_stores_only_an_argon2id_phc_hash() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));

    store.initialize("senha-segura", Hotkey::default()).unwrap();

    let serialized = fs::read_to_string(store.path()).unwrap();
    assert!(!serialized.contains("senha-segura"));
    assert!(serialized.contains("$argon2id$"));
    assert!(store.verify_password("senha-segura").unwrap());
    assert!(!store.verify_password("senha-incorreta").unwrap());
}

#[test]
fn password_accepts_empty_short_and_arbitrary_characters() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));

    store.initialize("", Hotkey::default()).unwrap();
    assert!(store.verify_password("").unwrap());

    store.change_password("", "1").unwrap();
    assert!(store.verify_password("1").unwrap());

    store.change_password("1", " ç日本語!@#").unwrap();
    assert!(store.verify_password(" ç日本語!@#").unwrap());
}

#[test]
fn password_is_limited_to_128_characters() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));

    assert_eq!(
        store
            .initialize(&"x".repeat(129), Hotkey::default())
            .unwrap_err(),
        ConfigError::InvalidPasswordLength
    );
}

#[test]
fn password_change_requires_the_current_password_and_is_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    store.initialize("senha-antiga", Hotkey::default()).unwrap();

    assert_eq!(
        store
            .change_password("senha-errada", "senha-nova")
            .unwrap_err(),
        ConfigError::AuthenticationFailed
    );
    assert!(store.verify_password("senha-antiga").unwrap());

    store.change_password("senha-antiga", "senha-nova").unwrap();
    assert!(!store.verify_password("senha-antiga").unwrap());
    assert!(store.verify_password("senha-nova").unwrap());
    assert!(!temp.path().join("config.json.tmp").exists());
}

#[test]
fn corrupted_hash_is_reported_instead_of_treated_as_a_wrong_password() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    let config = AppConfig {
        password_hash: "not-a-phc-hash".into(),
        ..AppConfig::for_test()
    };
    fs::write(store.path(), serde_json::to_vec(&config).unwrap()).unwrap();

    assert_eq!(
        store.verify_password("qualquer-senha").unwrap_err(),
        ConfigError::CorruptPasswordHash
    );
}

#[test]
fn default_hotkey_is_ctrl_shift_l() {
    assert_eq!(Hotkey::default().display_name(), "Ctrl+Shift+L");
}

#[test]
fn legacy_configuration_loads_with_new_visual_options_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    fs::write(
        store.path(),
        r#"{
          "version": 1,
          "enabled": true,
          "hotkey": {"control": true, "alt": false, "shift": true, "key": "L"},
          "password_hash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$ZHVtbXk"
        }"#,
    )
    .unwrap();

    let config = store.load().unwrap();
    assert!(!config.windows_hello_enabled);
    assert_eq!(config.dimming_percentage, 0);
    assert_eq!(config.unlock_message, "Digite a senha para desbloquear");
}

#[test]
fn unlock_message_is_persisted_and_validated() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    store.initialize("", Hotkey::default()).unwrap();

    let mut config = store.load().unwrap();
    config.unlock_message = "Acesso restrito".into();
    store.save(&config).unwrap();
    assert_eq!(store.load().unwrap().unlock_message, "Acesso restrito");

    config.unlock_message = "linha 1\nlinha 2".into();
    assert!(matches!(
        store.save(&config).unwrap_err(),
        ConfigError::InvalidConfig(_)
    ));
}

#[test]
fn dimming_percentage_is_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    store.initialize("", Hotkey::default()).unwrap();

    let mut config = store.load().unwrap();
    config.dimming_percentage = 65;
    store.save(&config).unwrap();

    assert_eq!(store.load().unwrap().dimming_percentage, 65);
}

#[test]
fn dimming_percentage_above_one_hundred_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    let config = AppConfig {
        dimming_percentage: 101,
        ..AppConfig::for_test()
    };
    fs::write(store.path(), serde_json::to_vec(&config).unwrap()).unwrap();

    assert!(matches!(
        store.load().unwrap_err(),
        ConfigError::InvalidConfig(_)
    ));
}

#[test]
fn invalid_widget_geometry_is_rejected_by_the_store() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path().join("config.json"));
    let mut config = AppConfig::for_test();
    config.widget.width = 5000;
    assert!(matches!(
        store.save(&config),
        Err(ConfigError::InvalidConfig(_))
    ));
}
