use std::collections::VecDeque;
use std::time::{Duration, Instant};

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);
const CRASH_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    RestartAgent { locked: bool },
    LockWindows,
}

#[derive(Debug)]
pub struct Watchdog {
    last_heartbeat: Instant,
    locked: bool,
    failures: VecDeque<Instant>,
}

impl Watchdog {
    pub fn new(now: Instant) -> Self {
        Self {
            last_heartbeat: now,
            locked: false,
            failures: VecDeque::new(),
        }
    }

    pub fn heartbeat(&mut self, now: Instant, locked: bool) {
        self.last_heartbeat = now;
        self.locked = locked;
    }

    pub fn tick(&mut self, now: Instant) -> Option<WatchdogAction> {
        if now.duration_since(self.last_heartbeat) >= HEARTBEAT_TIMEOUT {
            self.last_heartbeat = now;
            Some(if self.locked {
                WatchdogAction::LockWindows
            } else {
                WatchdogAction::RestartAgent { locked: false }
            })
        } else {
            None
        }
    }

    pub fn agent_failed(&mut self, now: Instant) -> WatchdogAction {
        if self.locked {
            self.failures.clear();
            return WatchdogAction::LockWindows;
        }
        while self
            .failures
            .front()
            .is_some_and(|failure| now.duration_since(*failure) > CRASH_WINDOW)
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() >= 3 {
            self.failures.clear();
            WatchdogAction::LockWindows
        } else {
            WatchdogAction::RestartAgent { locked: false }
        }
    }
}
