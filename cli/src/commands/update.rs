//! `expensify update` — policies, tag approvers, expense rules, employees.

use anyhow::{Context, Result, bail};
use expensify::{
    CategoriesUpdate, EmployeeSource, PrimaryPolicyMode, ReportFieldsUpdate, TagApprover,
    TagCsvConfig, TagsUpdate,
};

use crate::cli::{
    GlobalArgs, PrimaryPolicyArg, TagApproversArgs, UpdateCommand, UpdateEmployeesArgs,
    UpdateExpenseRuleArgs, UpdateMode, UpdatePolicyArgs,
};
use crate::commands::client;
use crate::output::View;
use crate::spec::{
    CategorySpec, EmployeeSpec, ReportFieldSpec, TagLevelSpec, parse_pair, read_input_bytes,
    read_json,
};
use crate::view;

pub async fn run(command: UpdateCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        UpdateCommand::Policy(args) => policy(args, global).await,
        UpdateCommand::TagApprovers(args) => tag_approvers(args, global).await,
        UpdateCommand::ExpenseRule(args) => expense_rule(args, global).await,
        UpdateCommand::Employees(args) => employees(args, global).await,
    }
}

async fn policy(args: UpdatePolicyArgs, global: &GlobalArgs) -> Result<()> {
    if args.categories.is_none() && args.report_fields.is_none() && !has_tags(&args) {
        bail!("nothing to update: give --categories, --report-fields, --tags or --tags-csv");
    }

    let client = client(global)?;
    let mut action = client.update_policies(args.policy_ids.iter().map(String::as_str));

    if let Some(path) = &args.categories {
        let specs: Vec<CategorySpec> = read_json(path)?;
        let categories = specs.into_iter().map(CategorySpec::build);
        action = action.categories(match args.categories_mode {
            UpdateMode::Merge => CategoriesUpdate::merge(categories),
            UpdateMode::ReplaceAll => CategoriesUpdate::replace_all(categories),
        });
    }

    if let Some(path) = &args.report_fields {
        let specs: Vec<ReportFieldSpec> = read_json(path)?;
        let fields = specs.into_iter().map(ReportFieldSpec::build);
        action = action.report_fields(match args.report_fields_mode {
            UpdateMode::Merge => ReportFieldsUpdate::merge(fields),
            UpdateMode::ReplaceAll => ReportFieldsUpdate::replace_all(fields),
        });
    }

    if let Some(path) = &args.tags {
        let specs: Vec<TagLevelSpec> = read_json(path)?;
        action = action.tags(TagsUpdate::replace_all_inline(
            specs.into_iter().map(TagLevelSpec::build),
        ));
    } else if let Some(path) = &args.tags_csv {
        let data = read_input_bytes(path)?;
        action = action.tags(TagsUpdate::replace_all_csv(data, tag_csv_config(&args)?));
    }

    action
        .await
        .with_context(|| format!("updating {}", args.policy_ids.join(", ")))?;

    View::acknowledgement(
        "policies",
        format!("updated {}", args.policy_ids.join(", ")),
    )
    .print(global.output)
}

fn has_tags(args: &UpdatePolicyArgs) -> bool {
    args.tags.is_some() || args.tags_csv.is_some()
}

/// Expensify's `setRequired` is one flag for dependent levels and one per
/// level otherwise; the library makes the wrong pairing unrepresentable, so
/// this only has to route the flags.
fn tag_csv_config(args: &UpdatePolicyArgs) -> Result<TagCsvConfig> {
    let mut config = if args.tags_csv_dependent {
        match args.tags_csv_required.as_slice() {
            [] => TagCsvConfig::dependent(false),
            [required] => TagCsvConfig::dependent(*required),
            more => bail!(
                "dependent tag levels take one --tags-csv-required value, got {}",
                more.len()
            ),
        }
    } else {
        TagCsvConfig::independent(args.tags_csv_required.iter().copied())
    };
    if args.tags_csv_gl_codes {
        config = config.with_gl_codes();
    }
    if args.tags_csv_header_row {
        config = config.with_header_row();
    }
    if args.tags_csv_tsv {
        config = config.tsv();
    }
    Ok(config)
}

async fn tag_approvers(args: TagApproversArgs, global: &GlobalArgs) -> Result<()> {
    if args.assign.is_empty() && args.clear.is_empty() {
        bail!("give at least one --assign TAG=EMAIL or --clear TAG");
    }

    let mut approvers = Vec::with_capacity(args.assign.len() + args.clear.len());
    for raw in &args.assign {
        let (tag, email) = parse_pair(raw, "--assign")?;
        if email.is_empty() {
            bail!("--assign {raw} has no email; use --clear {tag} to remove an approver");
        }
        approvers.push(TagApprover::assign(tag, email));
    }
    approvers.extend(args.clear.iter().map(TagApprover::clear));

    let client = client(global)?;
    let count = approvers.len();
    client
        .set_tag_approvers(args.policy_id.as_str(), approvers)
        .await
        .with_context(|| format!("setting tag approvers on {}", args.policy_id))?;

    View::acknowledgement(
        "tag approvers",
        format!("updated {count} tag(s) on {}", args.policy_id),
    )
    .print(global.output)
}

async fn expense_rule(args: UpdateExpenseRuleArgs, global: &GlobalArgs) -> Result<()> {
    if args.tag.is_none() && args.default_billable.is_none() {
        bail!("an expense rule needs --tag or --default-billable");
    }

    let client = client(global)?;
    let mut action = client.update_expense_rule(
        args.policy_id.as_str(),
        &args.employee_email,
        expensify::RuleId(args.rule_id),
    );
    if let Some(tag) = &args.tag {
        action = action.tag(tag);
    }
    if let Some(billable) = args.default_billable {
        action = action.default_billable(billable);
    }
    action.await.context("updating the expense rule")?;

    View::acknowledgement("expense rules", format!("updated rule {}", args.rule_id))
        .print(global.output)
}

async fn employees(args: UpdateEmployeesArgs, global: &GlobalArgs) -> Result<()> {
    let specs: Vec<EmployeeSpec> = read_json(&args.file)?;
    if specs.is_empty() {
        bail!("{} contains no employees", args.file);
    }
    let employees = specs
        .into_iter()
        .map(EmployeeSpec::build)
        .collect::<Vec<_>>();

    let client = client(global)?;
    let mut action = client.update_employees(EmployeeSource::Inline(employees));
    if args.dry_run {
        action = action.dry_run();
    }
    if let Some(mode) = args.primary_policy {
        action = action.primary_policy(match mode {
            PrimaryPolicyArg::None => PrimaryPolicyMode::None,
            PrimaryPolicyArg::NewEmployees => PrimaryPolicyMode::NewEmployees,
            PrimaryPolicyArg::AllEmployees => PrimaryPolicyMode::AllEmployees,
        });
    }
    if args.no_approval_chain_fixes {
        action = action.no_approval_chain_fixes();
    }
    if args.first_level_managers_only {
        action = action.first_level_managers_only();
    }
    if args.skip_notification_emails {
        action = action.skip_notification_emails();
    }
    if let Some(recipients) = &args.email_on_finish {
        action = action.email_on_finish(recipients);
    }

    let outcome = action.await.context("updating employees")?;

    // `wide` is where the per-employee detail lives; the default is the
    // one-line summary.
    let (noun, headers, rows) = if global.output.is_wide() {
        (
            "skipped employees",
            vec!["EMAIL", "REASON"],
            outcome
                .skipped
                .iter()
                .map(|employee| vec![employee.email.clone(), employee.reason.clone()])
                .collect(),
        )
    } else {
        (
            "employee updates",
            vec!["UPDATED", "DRY RUN", "SKIPPED"],
            vec![vec![
                outcome.updated_count.to_string(),
                outcome.dry_run.to_string(),
                outcome.skipped.len().to_string(),
            ]],
        )
    };

    View::new(noun, headers, rows, view::employee_outcome(&outcome)).print(global.output)
}
