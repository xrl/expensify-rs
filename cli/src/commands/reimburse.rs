//! `expensify reimburse` — Approved to Reimbursed, the only transition
//! Expensify supports.

use anyhow::{Context, Result, bail};
use expensify::ReimburseTargets;
use serde_json::json;

use crate::cli::{GlobalArgs, ReimburseArgs, usage_error};
use crate::commands::client;
use crate::output::View;
use crate::view;

pub async fn run(args: ReimburseArgs, global: &GlobalArgs) -> Result<()> {
    if !args.anchor.report_ids.is_empty() && args.until.is_some() {
        usage_error("--until narrows --since; it does nothing with --report-id");
    }

    let mut targets = if !args.anchor.report_ids.is_empty() {
        ReimburseTargets::report_ids(args.anchor.report_ids.iter().map(String::as_str))
    } else if let Some(since) = args.anchor.since {
        ReimburseTargets::since(since)
    } else {
        bail!("one of --report-id or --since is required");
    };
    if let Some(until) = args.until {
        targets = targets.until(until);
    }

    let client = client(global)?;
    let mut action = client.mark_reports_reimbursed(targets);
    if let Some(source) = &args.payment_source {
        action = action.payment_source(source);
    }

    // The two modes have different output types, so the branch runs the
    // whole call rather than just flipping a flag.
    if args.tolerate_partial {
        let outcome = action
            .tolerate_partial()
            .await
            .context("marking reports reimbursed")?;
        let rows = outcome
            .updated
            .iter()
            .map(|id| vec![id.to_string(), "updated".to_owned(), String::new()])
            .chain(outcome.skipped.iter().map(|report| {
                vec![
                    report.report_id.to_string(),
                    "skipped".to_owned(),
                    report.reason.clone(),
                ]
            }))
            .chain(outcome.failed.iter().map(|report| {
                vec![
                    report.report_id.to_string(),
                    "failed".to_owned(),
                    report.reason.clone(),
                ]
            }))
            .collect();
        return View::new(
            "reports",
            vec!["REPORT ID", "RESULT", "REASON"],
            rows,
            view::reimburse_outcome(&outcome),
        )
        .print(global.output);
    }

    let updated = action.await.context("marking reports reimbursed")?;

    View::new(
        "reports",
        vec!["REPORT ID", "RESULT"],
        updated
            .iter()
            .map(|id| vec![id.to_string(), "updated".to_owned()])
            .collect(),
        json!({ "updated": updated.iter().map(|id| id.as_str()).collect::<Vec<_>>() }),
    )
    .print(global.output)
}
