use bloqueio_transparente::package::{FileTransaction, is_downgrade, retire_file};

#[test]
fn update_replaces_binary_without_changing_existing_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.exe");
    let config = dir.path().join("config.json");
    let settings = br#"{"password_hash":"unchanged","windows_hello_enabled":true}"#;
    std::fs::write(&app, b"old signed binary").unwrap();
    std::fs::write(&config, settings).unwrap();
    let mut transaction = FileTransaction::default();
    transaction.replace(&app, b"new signed binary").unwrap();
    transaction.commit();
    assert_eq!(std::fs::read(&app).unwrap(), b"new signed binary");
    assert_eq!(std::fs::read(&config).unwrap(), settings);
}

#[test]
fn failed_service_activation_restores_binary_and_cached_installer() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.exe");
    let cache = dir.path().join("Installer/setup.exe");
    std::fs::write(&app, b"old").unwrap();
    let mut transaction = FileTransaction::default();
    transaction.replace(&app, b"new").unwrap();
    transaction.replace(&cache, b"setup").unwrap();
    transaction.rollback().unwrap();
    assert_eq!(std::fs::read(app).unwrap(), b"old");
    assert!(!cache.exists());
}

#[test]
fn repair_recovers_a_missing_binary_and_keeps_password_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.exe");
    let config = dir.path().join("config.json");
    std::fs::write(&config, b"existing password and preferences").unwrap();
    let mut transaction = FileTransaction::default();
    transaction.replace(&app, b"bundled signed binary").unwrap();
    transaction.commit();
    assert_eq!(std::fs::read(app).unwrap(), b"bundled signed binary");
    assert_eq!(
        std::fs::read(config).unwrap(),
        b"existing password and preferences"
    );
}

#[test]
fn failed_fresh_install_removes_new_files() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.exe");
    {
        let mut transaction = FileTransaction::default();
        transaction.replace(&app, b"new").unwrap();
    }
    assert!(!app.exists());
}

#[test]
fn multiple_writes_restore_the_original_not_an_intermediate_version() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.exe");
    std::fs::write(&app, b"original").unwrap();
    let mut transaction = FileTransaction::default();
    transaction.replace(&app, b"first").unwrap();
    transaction.replace(&app, b"second").unwrap();
    transaction.rollback().unwrap();
    assert_eq!(std::fs::read(app).unwrap(), b"original");
}

#[test]
fn shortcut_failure_restores_previous_shortcut_and_binary() {
    let dir = tempfile::tempdir().unwrap();
    let shortcut = dir.path().join("app.lnk");
    std::fs::write(&shortcut, b"old shortcut").unwrap();
    let mut transaction = FileTransaction::default();
    transaction.track(&shortcut).unwrap();
    std::fs::remove_file(&shortcut).unwrap();
    transaction.rollback().unwrap();
    assert_eq!(std::fs::read(shortcut).unwrap(), b"old shortcut");
}

#[test]
fn version_comparison_prevents_downgrades_without_lexical_ordering() {
    assert!(is_downgrade("0.10.0", "0.9.0"));
    assert!(!is_downgrade("0.6.9", "0.7.0"));
    assert!(!is_downgrade("0.7.0.0", "0.7.0"));
}

#[test]
fn deferred_uninstall_does_not_delete_a_later_reinstallation() {
    let dir = tempfile::tempdir().unwrap();
    let installer = dir.path().join("setup.exe");
    std::fs::write(&installer, b"old installation").unwrap();
    let retired = retire_file(&installer).unwrap();
    assert_ne!(retired, installer);
    std::fs::write(&installer, b"reinstalled before reboot").unwrap();
    std::fs::remove_file(retired).unwrap(); // Deferred cleanup at reboot.
    assert_eq!(
        std::fs::read(installer).unwrap(),
        b"reinstalled before reboot"
    );
}
