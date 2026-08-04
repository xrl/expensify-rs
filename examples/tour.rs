//! The design doc's running example: month-end close.
//! Needs live Expensify credentials to do anything.

// `129_00` reads as dollars-and-cents, which is the point.
#![allow(clippy::inconsistent_digit_grouping)]

use expensify::{
    Client, Credentials, Expense, ExpenseTax, ExportFormat, ExportTemplate, Json, Money, PolicyId,
    ReimburseTargets, ReportId, ReportState, ReportsQuery,
};
use serde::Deserialize;
use time::macros::date;

/// Shape produced by the user's FreeMarker template (JSON output).
#[derive(Deserialize)]
struct ReportRow {
    report_id: ReportId,
    employee: String,
    total_cents: i64,
}

const TEMPLATE_SRC: &str = r#"[<#list reports as report>
  {"report_id": "${report.reportID}",
   "employee": "${report.accountEmail}",
   "total_cents": ${report.total}}<#if report_has_next>,</#if>
</#list>]"#;

async fn month_end_close(client: &Client, policy: PolicyId) -> Result<(), expensify::Error> {
    // 1. Export July's approved reports, typed by the template.
    let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed(TEMPLATE_SRC);

    let file = client
        .export_reports(
            &template,
            ReportsQuery::since(date!(2026 - 07 - 01))
                .until(date!(2026 - 08 - 01))
                .policy_ids([&policy])
                .not_yet_exported_as("acme-etl"),
        )
        .state(ReportState::Approved)
        // The default is csv for every marker, including Json<_>.
        .format(ExportFormat::Json)
        .mark_as_exported("acme-etl")
        .await?;

    // 2. Download: decodes straight into the template's row type.
    let rows: Vec<ReportRow> = client.download(&file).await?;
    for row in &rows {
        println!(
            "{}: {} cents ({})",
            row.report_id, row.total_cents, row.employee
        );
    }

    // 3. Book July's hosting bill as an expense.
    let created = client
        .create_expenses([Expense::new(
            "Cloud Hosting Inc",
            date!(2026 - 07 - 31),
            Money::new(129_00, "USD"),
        )
        .category("Infrastructure")
        .external_id("hosting-2026-07")
        .tax(ExpenseTax::new("id_TAX_OPTION_16"))])
        .await?;
    println!("created {} transactions", created.len());

    // 4. Mark the exported reports reimbursed; tolerate partial success.
    let outcome = client
        .mark_reports_reimbursed(ReimburseTargets::report_ids(
            rows.iter().map(|r| &r.report_id),
        ))
        .payment_source("ACME-AP")
        .tolerate_partial()
        .await?;
    for skip in &outcome.skipped {
        eprintln!("skipped {}: {}", skip.report_id, skip.reason);
    }

    // 5. Typed policy read: tax rates without an unwrap in sight.
    let policies = client
        .get_policies([&policy])
        .with_tax()
        .with_categories()
        .await?;
    let info = &policies[&policy];
    for cat in &info.categories {
        println!("category {} enabled={}", cat.name, cat.enabled);
    }
    if let Some(tax) = &info.tax {
        for rate in &tax.rates {
            println!("tax {} = {}%", rate.name, rate.rate);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Credentials::new(
        std::env::var("EXPENSIFY_PARTNER_USER_ID")?,
        std::env::var("EXPENSIFY_PARTNER_USER_SECRET")?,
    ));
    let policy = PolicyId::new(std::env::var("EXPENSIFY_POLICY_ID")?);
    month_end_close(&client, policy).await?;
    Ok(())
}
