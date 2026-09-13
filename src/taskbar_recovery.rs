/// Bounded recovery after an agent restart or Explorer recreation.
pub struct TaskbarRecovery {
    attempts_left: u8,
}

impl Default for TaskbarRecovery {
    fn default() -> Self {
        // The agent timer runs once per second. Keep retrying even after a
        // successful attempt because Explorer may reapply its previous state.
        Self { attempts_left: 10 }
    }
}

impl TaskbarRecovery {
    pub fn cancel(&mut self) {
        self.attempts_left = 0;
    }

    pub fn tick(&mut self, locked: bool, mut restore: impl FnMut()) {
        if locked {
            self.cancel();
        } else if self.attempts_left > 0 {
            self.attempts_left -= 1;
            restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_a_taskbar_that_becomes_available_after_startup() {
        let mut recovery = TaskbarRecovery::default();
        let mut visible = false;
        recovery.tick(false, || {}); // Explorer is still switching desktops.
        recovery.tick(false, || visible = true);
        assert!(visible, "taskbar remains hidden after Windows unlock");
    }

    #[test]
    fn retries_even_if_explorer_overwrites_the_first_restore() {
        let mut recovery = TaskbarRecovery::default();
        recovery.tick(false, || {});
        let mut visible = false; // Explorer reapplies its old hidden state.
        recovery.tick(false, || visible = true);
        assert!(visible);
    }

    #[test]
    fn new_transparent_lock_cancels_pending_restoration() {
        let mut recovery = TaskbarRecovery::default();
        recovery.tick(true, || panic!("must not reveal taskbar while locked"));
        recovery.tick(false, || panic!("cancelled recovery must stay cancelled"));
    }

    #[test]
    fn recovery_is_bounded_and_can_be_rearmed_for_explorer_restart() {
        let mut recovery = TaskbarRecovery::default();
        for _ in 0..20 {
            recovery.tick(false, || {});
        }
        recovery.tick(false, || panic!("must stop changing Explorer"));
        recovery = TaskbarRecovery::default();
        let mut restored = false;
        recovery.tick(false, || restored = true);
        assert!(restored);
    }
}
