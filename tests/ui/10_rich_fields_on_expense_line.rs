//! Misuse 10: attaching category/tag/etc. to a report-creator expense line.
//! That job's wire shape carries exactly four fields, so anything richer
//! would be accepted here and silently dropped upstream.

use expensify::{ExpenseLine, Money};
use time::macros::date;

fn main() {
    let _ = ExpenseLine::new(
        "Sushi Place",
        date!(2026 - 07 - 04),
        Money::new(4_250, "USD"),
    )
    .category("Meals");
}
