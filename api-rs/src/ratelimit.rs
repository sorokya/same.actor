//! Fixed-window, per-client-IP rate limiting.
//!
//! Hand-rolled rather than pulled from a crate because the semantics that matter here are
//! small and worth owning: key on the actual visitor (see [`crate::routes::client_ip`]),
//! return `429` with a truthful `Retry-After`, and never grow without bound.
//!
//! This replaces `express-rate-limit` with no `trust proxy` configured — which counted the
//! reverse proxy rather than the visitor, so every user shared one bucket (review H1).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Above this many tracked clients, expired buckets are swept. Keeps memory bounded even
/// under a spray of spoofed or many-homed clients.
const SWEEP_THRESHOLD: usize = 20_000;

#[derive(Debug)]
struct Bucket {
    started: Instant,
    count: u32,
}

pub struct RateLimiter {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit: limit.max(1),
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// `Ok(())` if the request may proceed, `Err(retry_after)` if not.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();

        // A poisoned mutex here would mean another thread panicked while holding it. The
        // data is a plain counter map, so recovering is strictly better than propagating.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if buckets.len() > SWEEP_THRESHOLD {
            let window = self.window;
            buckets.retain(|_, bucket| now.duration_since(bucket.started) < window);
        }

        let bucket = buckets.entry(ip).or_insert(Bucket {
            started: now,
            count: 0,
        });

        let elapsed = now.duration_since(bucket.started);
        if elapsed >= self.window {
            bucket.started = now;
            bucket.count = 0;
        }

        if bucket.count >= self.limit {
            return Err(self
                .window
                .saturating_sub(now.duration_since(bucket.started)));
        }

        bucket.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn the_limit_is_enforced_per_client() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));

        for _ in 0..3 {
            assert!(limiter.check(ip(1)).is_ok());
        }
        assert!(
            limiter.check(ip(1)).is_err(),
            "fourth request must be refused"
        );

        // One client exhausting the budget must not affect another. This is the property
        // the old deployment got wrong.
        assert!(limiter.check(ip(2)).is_ok());
        assert!(limiter.check(ip(2)).is_ok());
    }

    #[test]
    fn the_window_resets() {
        let limiter = RateLimiter::new(1, Duration::from_millis(20));
        assert!(limiter.check(ip(1)).is_ok());
        assert!(limiter.check(ip(1)).is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check(ip(1)).is_ok());
    }

    #[test]
    fn retry_after_is_within_the_window() {
        let limiter = RateLimiter::new(1, Duration::from_secs(5));
        assert!(limiter.check(ip(1)).is_ok());
        let retry_after = limiter.check(ip(1)).unwrap_err();
        assert!(retry_after <= Duration::from_secs(5));
        assert!(retry_after > Duration::ZERO);
    }
}
