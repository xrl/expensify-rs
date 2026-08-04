//! Misuse 3: an export template handed to the reconciliation job. The two
//! FreeMarker dialects evaluate against disjoint data models, so this is a
//! type error rather than garbage output.

use expensify::{Client, Credentials, ExportTemplate, ReconciliationScope};
use time::macros::date;

fn main() {
    let client = Client::new(Credentials::new("id", "secret"));
    let template = ExportTemplate::new("<#list reports as report></#list>");

    let _ = client.domain("acme.com").reconcile(
        &template,
        date!(2026 - 07 - 01),
        date!(2026 - 07 - 31),
        ReconciliationScope::All,
    );
}
