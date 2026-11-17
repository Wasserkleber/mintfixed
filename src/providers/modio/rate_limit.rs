//! Shared mod.io backoff: used only for API HTTP 429 responses.
//!
//! All client middleware instances use the same gate. Never hold the lock
//! while sleeping, so a later 429 can extend the cooldown.
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(super) const MAX_HTTP_RETRIES: u32 = 5;
pub(super) const MAX_DOWNLOAD_RESTARTS: u32 = 5;

#[derive(Clone, Copy, Debug)]
pub(super) struct RetryPolicy {
    pub(super) max_retries: u32,
    pub(super) base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: MAX_HTTP_RETRIES,
            base_delay: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    pub(super) fn delay(self, next_retry: u32, retry_after: Option<u64>) -> Duration {
        let backoff = self
            .base_delay
            .saturating_mul(1_u32 << next_retry.min(6))
            .min(Duration::from_secs(60));
        retry_after
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(backoff)
    }
}

#[derive(Default, Debug)]
pub(super) struct RateLimitGate {
    until: Mutex<Option<Instant>>,
}

impl RateLimitGate {
    fn extend_at(&self, now: Instant, duration: Duration) {
        let Some(deadline) = now.checked_add(duration) else {
            return;
        };
        let mut until = self
            .until
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if until.is_none_or(|previous| previous < deadline) {
            *until = Some(deadline);
        }
    }

    fn remaining_at(&self, now: Instant) -> Option<Duration> {
        let until = self
            .until
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        until
            .and_then(|deadline| deadline.checked_duration_since(now))
            .filter(|duration| !duration.is_zero())
    }

    pub(super) fn defer(&self, duration: Duration) {
        self.extend_at(Instant::now(), duration);
    }

    pub(super) async fn wait(&self) {
        while let Some(duration) = self.remaining_at(Instant::now()) {
            tokio::time::sleep(duration).await;
        }
    }
}

pub(super) fn global_rate_gate() -> Arc<RateLimitGate> {
    static GLOBAL: OnceLock<Arc<RateLimitGate>> = OnceLock::new();
    GLOBAL
        .get_or_init(|| Arc::new(RateLimitGate::default()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delays_preserve_existing_backoff_and_retry_after() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1, None), Duration::from_secs(2));
        assert_eq!(policy.delay(2, None), Duration::from_secs(4));
        assert_eq!(policy.delay(8, None), Duration::from_secs(60));
        assert_eq!(policy.delay(1, Some(17)), Duration::from_secs(17));
        assert_eq!(policy.delay(1, Some(0)), Duration::from_secs(2));
        assert_eq!(policy.max_retries, 5);
        assert_eq!(MAX_DOWNLOAD_RESTARTS, 5);
    }

    #[test]
    fn shared_gate_keeps_longest_deadline_and_expires() {
        let gate = RateLimitGate::default();
        let now = Instant::now();
        gate.extend_at(now, Duration::from_secs(10));
        gate.extend_at(now, Duration::from_secs(2));
        assert_eq!(
            gate.remaining_at(now + Duration::from_secs(3)),
            Some(Duration::from_secs(7))
        );
        gate.extend_at(now + Duration::from_secs(4), Duration::from_secs(11));
        assert_eq!(
            gate.remaining_at(now + Duration::from_secs(7)),
            Some(Duration::from_secs(8))
        );
        assert!(gate.remaining_at(now + Duration::from_secs(16)).is_none());
    }

    #[test]
    fn global_gate_is_shared_across_clients() {
        let first = global_rate_gate();
        let second = global_rate_gate();
        assert!(Arc::ptr_eq(&first, &second));
    }
}
