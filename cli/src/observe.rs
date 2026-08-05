//! The library's observer hook, bridged onto `tracing`.
//!
//! The library deliberately has no `tracing` dependency — it hands over a
//! typed [`Exchange`] and lets the binary decide what a log line is. This is
//! that decision: a summary at INFO (`-v`), the full bodies at DEBUG (`-vv`).

use expensify::{Exchange, ObservedRequest, Observer};

pub struct Tracing;

impl Observer for Tracing {
    fn on_request(&self, request: &ObservedRequest) {
        tracing::info!(job = request.job_type(), url = %request.url(), "request");
        // Emitted separately from the summary so `-v` stays one line per call.
        tracing::debug!("request as sent (credentials redacted):\n{request}");
    }

    fn on_exchange(&self, exchange: &Exchange) {
        tracing::info!(
            status = exchange.status().as_u16(),
            content_type = exchange.content_type().unwrap_or("(none)"),
            bytes = exchange.body().len(),
            elapsed_ms = exchange.duration().as_millis(),
            "response"
        );
        tracing::debug!("response body:\n{}", exchange.body_text());
    }
}
