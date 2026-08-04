//! Misuse 7: an export with no filters, which Expensify rejects with a 410.
//! `ReportsQuery` has only anchored constructors — there is no `default()`.

use expensify::ReportsQuery;

fn main() {
    let _ = ReportsQuery::default();
}
