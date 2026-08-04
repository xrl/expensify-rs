//! Misuse 18: an exporter-only output format on the reconciliation job,
//! which accepts only csv/txt/json/xml. Same split as misuse 13: one wire
//! key, two vocabularies, so the narrower one is its own type.

use expensify::{Client, Credentials, ExportFormat, ReconciliationScope, ReconciliationTemplate};
use time::macros::date;

fn main() {
    let client = Client::new(Credentials::new("id", "secret"));
    let template = ReconciliationTemplate::new("<#list cards as card, reports></#list>");
    let _ = client
        .domain("acme.com")
        .reconcile(
            &template,
            date!(2026 - 07 - 01),
            date!(2026 - 07 - 31),
            ReconciliationScope::All,
        )
        .format(ExportFormat::Xlsx);
}
