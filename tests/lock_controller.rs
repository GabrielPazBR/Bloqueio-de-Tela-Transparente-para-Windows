use bloqueio_transparente::lock::{Action, Event, LockController, LockState, UnlockMethod};
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
fn windows_hello_is_the_only_unlock_action_when_enabled() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.set_unlock_method(UnlockMethod::WindowsHello);
    controller.handle(Event::LockRequested, now);

    let actions = controller.handle(Event::PrintableCharacter('s'), now);

    assert_eq!(controller.state(), LockState::Verifying);
    assert!(controller.password_buffer().is_empty());
    assert_eq!(
        actions,
        vec![Action::ShowPasswordPrompt, Action::VerifyWindowsHello]
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::VerifyPassword(_)))
    );
}

#[test]
fn mouse_interaction_requests_windows_hello_once() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.set_unlock_method(UnlockMethod::WindowsHello);
    controller.handle(Event::LockRequested, now);

    let first = controller.handle(Event::UserInteraction, now);
    let duplicate = controller.handle(Event::UserInteraction, now);

    assert_eq!(
        first,
        vec![Action::ShowPasswordPrompt, Action::VerifyWindowsHello]
    );
    assert!(duplicate.is_empty());
}

#[test]
fn canceled_windows_hello_keeps_the_screen_locked_and_restores_hooks() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.set_unlock_method(UnlockMethod::WindowsHello);
    controller.handle(Event::LockRequested, now);
    controller.handle(Event::UserInteraction, now);

    let actions = controller.handle(Event::AlternativeAuthenticationRejected, now);

    assert_eq!(controller.state(), LockState::Locked);
    assert_eq!(
        actions,
        vec![Action::InstallInputHooks, Action::ShowPasswordError]
    );
}

#[test]
fn inactive_password_prompt_closes_without_unlocking() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.handle(Event::LockRequested, now);
    controller.handle(Event::PrintableCharacter('s'), now);

    let actions = controller.handle(
        Event::PromptInactivityElapsed,
        now + Duration::from_secs(15),
    );

    assert_eq!(controller.state(), LockState::Locked);
    assert!(controller.password_buffer().is_empty());
    assert_eq!(actions, vec![Action::HidePasswordPrompt]);
}

#[test]
fn inactive_windows_hello_is_canceled_without_unlocking() {
    let now = Instant::now();
    let mut controller = LockController::new();
    controller.set_unlock_method(UnlockMethod::WindowsHello);
    controller.handle(Event::LockRequested, now);
    controller.handle(Event::UserInteraction, now);

    let actions = controller.handle(
        Event::PromptInactivityElapsed,
        now + Duration::from_secs(15),
    );

    assert_eq!(controller.state(), LockState::Locked);
    assert_eq!(
        actions,
        vec![
            Action::CancelWindowsHello,
            Action::HidePasswordPrompt,
            Action::InstallInputHooks,
        ]
    );
    assert!(
        controller
            .handle(Event::AlternativeAuthenticationRejected, now)
            .is_empty()
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
