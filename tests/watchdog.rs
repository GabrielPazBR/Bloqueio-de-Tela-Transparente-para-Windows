use bloqueio_transparente::watchdog::{Watchdog, WatchdogAction};
use std::time::{Duration, Instant};

#[test]
fn missing_heartbeat_for_six_seconds_restarts_the_agent_locked() {
    let start = Instant::now();
    let mut watchdog = Watchdog::new(start);
    watchdog.heartbeat(start, true);

    assert_eq!(watchdog.tick(start + Duration::from_secs(5)), None);
    assert_eq!(
        watchdog.tick(start + Duration::from_secs(6)),
        Some(WatchdogAction::RestartAgent { locked: true })
    );
}

#[test]
fn third_failure_in_sixty_seconds_uses_windows_lock() {
    let start = Instant::now();
    let mut watchdog = Watchdog::new(start);

    assert_eq!(
        watchdog.agent_failed(start),
        WatchdogAction::RestartAgent { locked: false }
    );
    assert_eq!(
        watchdog.agent_failed(start + Duration::from_secs(20)),
        WatchdogAction::RestartAgent { locked: false }
    );
    assert_eq!(
        watchdog.agent_failed(start + Duration::from_secs(40)),
        WatchdogAction::LockWindows
    );
}

#[test]
fn old_failures_do_not_count_toward_the_crash_window() {
    let start = Instant::now();
    let mut watchdog = Watchdog::new(start);
    watchdog.agent_failed(start);
    watchdog.agent_failed(start + Duration::from_secs(61));

    assert_eq!(
        watchdog.agent_failed(start + Duration::from_secs(62)),
        WatchdogAction::RestartAgent { locked: false }
    );
}
