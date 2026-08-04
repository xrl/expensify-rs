//! `expensify export` — the two jobs that render a file server-side.

use anyhow::{Context, Result, bail};
use expensify::{
    ExportFormat, ExportTemplate, ExportedFile, OnFinish, ReconciliationFormat,
    ReconciliationScope, ReportState, ReportsQuery,
};
use serde_json::json;

use crate::cli::{
    ExportCommand, ExportFormatArg, ExportReportsArgs, GlobalArgs, ReconcileArgs,
    ReconciliationFormatArg, ReconciliationScopeArg, ReportStateArg, usage_error,
};
use crate::commands::{client, note};
use crate::output::View;
use crate::spec::read_input;

pub async fn run(command: ExportCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        ExportCommand::Reports(args) => reports(args, global).await,
        ExportCommand::Reconciliation(args) => reconciliation(args, global).await,
    }
}

async fn reports(args: ExportReportsArgs, global: &GlobalArgs) -> Result<()> {
    let anchor = &args.anchor;
    if !anchor.report_ids.is_empty() && args.until.is_some() {
        usage_error(
            "--until narrows --since or --approved-after; it does nothing with --report-id",
        );
    }

    let mut query = if !anchor.report_ids.is_empty() {
        ReportsQuery::report_ids(anchor.report_ids.iter().map(String::as_str))
    } else if let Some(since) = anchor.since {
        ReportsQuery::since(since)
    } else if let Some(approved_after) = anchor.approved_after {
        ReportsQuery::approved_after(approved_after)
    } else {
        // clap's required group already refuses this.
        bail!("one of --report-id, --since or --approved-after is required");
    };
    if let Some(until) = args.until {
        query = query.until(until);
    }
    if !args.policy_ids.is_empty() {
        query = query.policy_ids(args.policy_ids.iter().map(String::as_str));
    }
    if let Some(label) = &args.not_exported_as {
        query = query.not_yet_exported_as(label);
    }

    let source = read_input(&args.template).context("reading the template")?;
    let template = ExportTemplate::new(source);

    let client = client(global)?;
    let mut action = client
        .export_reports(&template, query)
        .format(match args.format {
            ExportFormatArg::Csv => ExportFormat::Csv,
            ExportFormatArg::Xls => ExportFormat::Xls,
            ExportFormatArg::Xlsx => ExportFormat::Xlsx,
            ExportFormatArg::Txt => ExportFormat::Txt,
            ExportFormatArg::Json => ExportFormat::Json,
            ExportFormatArg::Xml => ExportFormat::Xml,
        });
    for state in &args.states {
        action = action.state(match state {
            ReportStateArg::Open => ReportState::Open,
            ReportStateArg::Submitted => ReportState::Submitted,
            ReportStateArg::Approved => ReportState::Approved,
            ReportStateArg::Reimbursed => ReportState::Reimbursed,
            ReportStateArg::Archived => ReportState::Archived,
        });
    }
    if let Some(limit) = args.limit {
        action = action.limit(limit);
    }
    if let Some(email) = &args.employee_email {
        action = action.employee_email(email);
    }
    if let Some(basename) = &args.basename {
        action = action.file_basename(basename);
    }
    if let Some(label) = &args.mark_as_exported {
        action = action.mark_as_exported(label);
    }
    if let Some(recipients) = &args.email {
        let mut email = OnFinish::email(recipients);
        if let Some(message) = &args.email_message {
            email = email.message(message);
        }
        action = action.on_finish(email);
    }
    if args.test_run {
        action = action.test_run();
    }

    let file = action.await.context("starting the report export")?;
    note(
        global,
        "Export queued. Rendering continues server-side; retry `expensify download` \
         if the file is not ready yet.",
    );
    print_handle(&file, global)
}

async fn reconciliation(args: ReconcileArgs, global: &GlobalArgs) -> Result<()> {
    let source = read_input(&args.template).context("reading the template")?;
    let template = expensify::ReconciliationTemplate::new(source);

    let client = client(global)?;
    let mut action = client
        .domain(&args.domain)
        .reconcile(
            &template,
            args.start,
            args.end,
            match args.scope {
                ReconciliationScopeArg::Unreported => ReconciliationScope::Unreported,
                ReconciliationScopeArg::All => ReconciliationScope::All,
            },
        )
        .format(match args.format {
            ReconciliationFormatArg::Csv => ReconciliationFormat::Csv,
            ReconciliationFormatArg::Txt => ReconciliationFormat::Txt,
            ReconciliationFormatArg::Json => ReconciliationFormat::Json,
            ReconciliationFormatArg::Xml => ReconciliationFormat::Xml,
        });
    if let Some(feed) = &args.feed {
        action = action.feed(feed);
    }
    if let Some(recipients) = &args.email_on_finish {
        action = action.email_on_finish(recipients);
    }

    let file = action
        .await
        .with_context(|| format!("reconciling {}", args.domain))?;
    print_handle(&file, global)
}

/// Both jobs answer with a file handle; `expensify download` takes the two
/// columns printed here.
fn print_handle<F>(file: &ExportedFile<F>, global: &GlobalArgs) -> Result<()> {
    let file_system = match file.file_system() {
        expensify::FileSystem::IntegrationServer => "integration-server",
        expensify::FileSystem::Reconciliation => "reconciliation",
    };
    View::new(
        "files",
        vec!["FILENAME", "FILE SYSTEM"],
        vec![vec![file.name().to_owned(), file_system.to_owned()]],
        json!({ "filename": file.name(), "file_system": file_system }),
    )
    .print(global.output)
}
