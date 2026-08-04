//! Misuse 16: the reverse of case 3 — a reconciliation template handed to
//! the report exporter. Neither direction of the cross-wiring compiles.

use expensify::{Client, Credentials, ReconciliationTemplate, ReportsQuery};

fn main() {
    let client = Client::new(Credentials::new("id", "secret"));
    let template = ReconciliationTemplate::new("<#list cards as card, reports></#list>");

    let _ = client.export_reports(&template, ReportsQuery::report_ids(["R1"]));
}
