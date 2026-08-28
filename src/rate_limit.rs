use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed,
    RetryAfter(u32),
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    failed_attempts: u32,
    retry_at: Option<Instant>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&mut self, now: Instant) -> RateLimitDecision {
        match self.retry_at {
            Some(deadline) if now < deadline => {
                RateLimitDecision::RetryAfter(deadline.duration_since(now).as_secs().max(1) as u32)
            }
            Some(_) => {
                self.retry_at = None;
                RateLimitDecision::Allowed
            }
            None => RateLimitDecision::Allowed,
        }
    }

    pub fn record_failure(&mut self, now: Instant) -> u32 {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        let delay = if self.failed_attempts >= 5 {
            let shift = (self.failed_attempts - 4).min(5);
            (1_u32 << shift).min(30)
        } else {
            0
        };
        self.retry_at = (delay > 0).then_some(now + Duration::from_secs(delay.into()));
        delay
    }

    pub fn record_success(&mut self) {
        self.failed_attempts = 0;
        self.retry_at = None;
    }
}
