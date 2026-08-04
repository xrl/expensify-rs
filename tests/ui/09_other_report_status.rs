//! Misuse 9: any report-status value other than REIMBURSED. There is no
//! status parameter to hold one — the verb is the status.

use expensify::{Client, Credentials, ReimburseTargets};

fn main() {
    let client = Client::new(Credentials::new("id", "secret"));

    let _ = client
        .mark_reports_reimbursed(ReimburseTargets::report_ids(["R1"]))
        .status("APPROVED");
}
