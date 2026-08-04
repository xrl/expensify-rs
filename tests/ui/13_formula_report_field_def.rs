//! Misuse 13: defining a formula report field through the updater, which
//! Expensify rejects. The getter's `ReportFieldType` has a `Formula`
//! variant because the getter emits one; `ReportFieldDef` takes the
//! narrower `ReportFieldDefType`, which has no such value to pass.

use expensify::{ReportFieldDef, ReportFieldType};

fn main() {
    let _ = ReportFieldDef::new("Title", ReportFieldType::Formula);
}
