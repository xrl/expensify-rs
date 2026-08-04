//! Misuse 8: assuming everything was reimbursed. The strict path resolves to
//! `Vec<ReportId>` — there is no `skipped` list to forget to check, and an
//! actual 207 is an error. Reaching the outcome requires `tolerate_partial()`.

use expensify::{Client, Credentials, Error, ReimburseTargets};

async fn run(client: &Client) -> Result<(), Error> {
    let outcome = client
        .mark_reports_reimbursed(ReimburseTargets::report_ids(["R1"]))
        .await?;

    let _ = outcome.skipped;
    Ok(())
}

fn main() {
    let _ = run(&Client::new(Credentials::new("id", "secret")));
}
