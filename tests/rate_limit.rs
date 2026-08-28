use bloqueio_transparente::rate_limit::{RateLimitDecision, RateLimiter};
use std::time::{Duration, Instant};

#[test]
fn fifth_failure_starts_a_progressive_time_limit_capped_at_thirty_seconds() {
    let mut limiter = RateLimiter::new();
    let mut now = Instant::now();

    for _ in 0..4 {
        assert_eq!(limiter.record_failure(now), 0);
    }
    assert_eq!(limiter.record_failure(now), 2);
    assert_eq!(limiter.check(now), RateLimitDecision::RetryAfter(2));

    now += Duration::from_secs(2);
    for expected in [4, 8, 16, 30, 30] {
        assert_eq!(limiter.check(now), RateLimitDecision::Allowed);
        assert_eq!(limiter.record_failure(now), expected);
        now += Duration::from_secs(expected.into());
    }
}

#[test]
fn successful_authentication_resets_the_limit() {
    let mut limiter = RateLimiter::new();
    let now = Instant::now();
    for _ in 0..5 {
        limiter.record_failure(now);
    }

    limiter.record_success();

    assert_eq!(limiter.check(now), RateLimitDecision::Allowed);
    assert_eq!(limiter.record_failure(now), 0);
}
