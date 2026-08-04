//! The default rate limiter: two governor windows, both awaited per send.
//!
//! Expensify publishes 5 requests/10 s and 20 requests/60 s. Modelling only
//! the tighter window would still burn the minute budget in 40 s, so both
//! run and a send waits on whichever is currently binding.

use std::num::NonZeroU32;
use std::time::Duration;

use governor::{Quota, RateLimiter};

use crate::client::RateGate;

impl RateGate {
    pub(crate) fn new() -> Self {
        Self {
            per_10s: RateLimiter::direct(quota(Duration::from_secs(10), 5)),
            per_60s: RateLimiter::direct(quota(Duration::from_secs(60), 20)),
        }
    }

    pub(crate) async fn acquire(&self) {
        self.per_10s.until_ready().await;
        self.per_60s.until_ready().await;
    }
}

/// `burst` cells per `window`, replenishing one cell every `window / burst`
/// — governor's way of spelling a sliding budget.
fn quota(window: Duration, burst: u32) -> Quota {
    let burst = NonZeroU32::new(burst).expect("burst is non-zero");
    Quota::with_period(window / burst.get())
        .expect("replenish period is non-zero")
        .allow_burst(burst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn burst_is_immediate_then_throttles() {
        let gate = RateGate::new();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            gate.acquire().await;
        }
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "burst blocked"
        );

        // The sixth send has to wait for the 10 s window to replenish a cell.
        assert!(gate.per_10s.check().is_err());
    }
}
