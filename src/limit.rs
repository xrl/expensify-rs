//! The default rate limiter: two governor windows, both awaited per send.
//!
//! Expensify publishes 5 requests/10 s and 20 requests/60 s. Modelling only
//! the tighter window would still burn the minute budget in 40 s, so both
//! run and a send waits on whichever is currently binding.

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

/// GCRA admits `burst + elapsed / period` cells, so the obvious
/// `with_period(window / budget).allow_burst(budget)` spelling admits the
/// burst *on top of* a full window's replenishment — roughly double the
/// published rate on a cold start. Keeping the implicit burst of one and
/// spreading the remaining `budget - 1` cells across the window makes
/// `budget` per `window` an upper bound at every offset, counting both
/// endpoints. The cost is that only the first send is instant; the
/// alternative is a 429, which is worse.
fn quota(window: Duration, budget: u32) -> Quota {
    debug_assert!(budget >= 2, "a budget below 2 leaves nothing to pace");
    Quota::with_period(window / (budget - 1)).expect("replenish period is non-zero")
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::clock::{Clock, FakeRelativeClock};

    /// Cell admission times for a client that sends as fast as the limiter
    /// allows, over `span`, on a fake clock.
    fn greedy_sends(window: Duration, budget: u32, span: Duration) -> Vec<Duration> {
        let clock = FakeRelativeClock::default();
        let limiter = RateLimiter::direct_with_clock(quota(window, budget), clock.clone());
        let mut now = Duration::ZERO;
        let mut sends = Vec::new();
        while now <= span {
            match limiter.check() {
                Ok(_) => sends.push(now),
                Err(negative) => {
                    let wait = negative.wait_time_from(clock.now());
                    clock.advance(wait);
                    now += wait;
                }
            }
        }
        sends
    }

    /// Most cells admitted in any `window`-long span, counting both
    /// endpoints. This is what Expensify enforces, and what a burst
    /// allowance on top of a full window's replenishment quietly breaks.
    fn peak_per_window(window: Duration, budget: u32) -> usize {
        let sends = greedy_sends(window, budget, window * 4);
        sends
            .iter()
            .map(|start| {
                sends
                    .iter()
                    .filter(|t| **t >= *start && **t <= *start + window)
                    .count()
            })
            .max()
            .expect("at least one send")
    }

    #[test]
    fn sustained_rate_respects_both_documented_budgets() {
        for (window, budget) in [(Duration::from_secs(10), 5), (Duration::from_secs(60), 20)] {
            let peak = peak_per_window(window, budget);
            assert!(
                peak <= budget as usize,
                "{budget} per {window:?} exceeded: {peak}"
            );
            // The other direction too: a limiter that admits almost nothing
            // would satisfy the bound above and be useless.
            assert!(
                peak >= budget as usize,
                "{budget} per {window:?} under-used: {peak}"
            );
        }
    }

    /// Both windows are enforced, so the 60 s budget binds and the 10 s one
    /// is never approached.
    #[test]
    fn the_minute_budget_is_the_binding_one() {
        let per_minute = greedy_sends(Duration::from_secs(60), 20, Duration::from_secs(60));
        let gaps: Vec<Duration> = per_minute.windows(2).map(|p| p[1] - p[0]).collect();
        assert!(
            gaps.iter().all(|gap| *gap > Duration::from_secs(10) / 5),
            "minute window must be slower than the 10 s window"
        );
    }

    #[tokio::test]
    async fn first_send_is_immediate_and_the_second_waits() {
        let gate = RateGate::new();
        let start = std::time::Instant::now();
        gate.acquire().await;
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "first blocked"
        );

        // The 60 s window is the binding one: ~3.2 s between sends.
        assert!(gate.per_60s.check().is_err());
    }
}
