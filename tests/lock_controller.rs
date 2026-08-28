use bloqueio_transparente::lock::{Action, Event, LockController, LockState};
use std::time::{Duration, Instant};

#[test]
fn lock_request_covers_all_monitors_and_blocks_input() {
    let now = Instant::now();
    let mut controller = LockController::new();

    let actions = controller.handle(Event::LockRequested, now);

    assert_eq!(controller.state(), LockState::Locked);
    assert_eq!(
        actions,
        vec![Action::ShowOverlays, Action::InstallInputHooks]
    );
}

#[test]
fn first_printable_character_opens_prompt_and_is_preserved() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::PrintableCharacter('s'), now);

    assert_eq!(controller.state(), LockState::Prompting);
    assert_eq!(controller.password_buffer(), "s");
    assert_eq!(actions, vec![Action::ShowPasswordPrompt]);
}

#[test]
fn enter_while_locked_submits_an_empty_password() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::SubmitPassword, now);

    assert_eq!(controller.state(), LockState::Verifying);
    assert_eq!(
        actions,
        vec![
            Action::ShowPasswordPrompt,
            Action::VerifyPassword(String::new())
        ]
    );
}

#[test]
fn correct_password_releases_hooks_and_overlays() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);
    controller.handle(Event::PrintableCharacter('s'), now);
    controller.handle(Event::SubmitPassword, now);

    let actions = controller.handle(Event::PasswordAccepted, now);

    assert_eq!(controller.state(), LockState::Ready);
    assert_eq!(
        actions,
        vec![Action::RemoveInputHooks, Action::HideOverlays]
    );
}

#[test]
fn accepted_alternative_authentication_releases_a_locked_screen() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::AlternativeAuthenticationAccepted, now);

    assert_eq!(controller.state(), LockState::Ready);
    assert_eq!(
        actions,
        vec![Action::RemoveInputHooks, Action::HideOverlays]
    );
}

#[test]
fn password_failures_show_an_error_and_obey_the_service_delay() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    for _ in 0..4 {
        controller.handle(Event::PrintableCharacter('x'), now);
        controller.handle(Event::SubmitPassword, now);
        let actions = controller.handle(
            Event::PasswordRejected {
                retry_after_seconds: 0,
            },
            now,
        );
        assert_eq!(actions, vec![Action::ShowPasswordError]);
    }
    controller.handle(Event::PrintableCharacter('x'), now);
    controller.handle(Event::SubmitPassword, now);
    controller.handle(
        Event::PasswordRejected {
            retry_after_seconds: 30,
        },
        now,
    );
    assert_eq!(controller.failed_attempts(), 5);
    assert_eq!(controller.retry_at(), Some(now + Duration::from_secs(30)));
}

#[test]
fn server_retry_time_is_authoritative_for_the_lock_prompt() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);
    controller.handle(Event::PrintableCharacter('x'), now);
    controller.handle(Event::SubmitPassword, now);

    controller.handle(
        Event::PasswordRejected {
            retry_after_seconds: 12,
        },
        now,
    );

    assert_eq!(controller.retry_at(), Some(now + Duration::from_secs(12)));
}

#[test]
fn display_change_rebuilds_overlays_without_unlocking() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::DisplayChanged, now);

    assert_eq!(controller.state(), LockState::Locked);
    assert_eq!(actions, vec![Action::RebuildOverlays]);
}

#[test]
fn corrupt_configuration_while_locked_falls_back_to_windows_lock() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::ConfigurationCorrupt, now);

    assert_eq!(controller.state(), LockState::FallbackWindowsLock);
    assert_eq!(actions, vec![Action::LockWindows]);
}
