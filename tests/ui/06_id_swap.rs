//! Misuse 6: passing a `PolicyId` where a `ReportId` belongs. Every surface
//! takes `impl Into<XxxId>`, so literals work but a sibling id type does not.

use expensify::{Expense, Money, PolicyId};
use time::macros::date;

fn main() {
    let policy = PolicyId::new("0123456789ABCDEF");

    let _ = Expense::new(
        "Cloud Hosting Inc",
        date!(2026 - 07 - 31),
        Money::new(12_900, "USD"),
    )
    .report_id(policy);
}
