//! `expensify create` — policies, expenses, reports and expense rules.

use anyhow::{Context, Result, bail};
use expensify::{Expense, Money, PolicyPlan};
use serde_json::Value;

use crate::cli::{
    CreateCommand, CreateExpenseRuleArgs, CreateExpensesArgs, CreatePolicyArgs, CreateReportArgs,
    GlobalArgs, PolicyPlanArg, usage_error,
};
use crate::commands::client;
use crate::output::View;
use crate::spec::{ExpenseLineSpec, ExpenseSpec, parse_pair, read_json};
use crate::view;

pub async fn run(command: CreateCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        CreateCommand::Policy(args) => policy(args, global).await,
        CreateCommand::Expenses(args) => expenses(args, global).await,
        CreateCommand::Report(args) => report(args, global).await,
        CreateCommand::ExpenseRule(args) => expense_rule(args, global).await,
    }
}

async fn policy(args: CreatePolicyArgs, global: &GlobalArgs) -> Result<()> {
    let client = client(global)?;
    let mut action = client.create_policy(&args.name);
    if let Some(plan) = args.plan {
        action = action.plan(match plan {
            PolicyPlanArg::Team => PolicyPlan::Team,
            PolicyPlanArg::Corporate => PolicyPlan::Corporate,
        });
    }

    let created = action.await.context("creating the policy")?;

    View::new(
        "policies",
        vec!["POLICY ID", "NAME"],
        vec![vec![created.policy_id.to_string(), created.name.clone()]],
        view::created_policy(&created),
    )
    .print(global.output)
}

async fn expenses(args: CreateExpensesArgs, global: &GlobalArgs) -> Result<()> {
    let expenses = match (&args.file, &args.merchant) {
        (Some(path), _) => {
            let specs: Vec<ExpenseSpec> = read_json(path)?;
            if specs.is_empty() {
                bail!("{path} contains no expenses");
            }
            specs
                .into_iter()
                .map(ExpenseSpec::build)
                .collect::<Result<Vec<_>>>()?
        }
        (None, Some(_)) => vec![inline_expense(&args)?],
        (None, None) => usage_error("give --file, or --merchant with --date and --amount-cents"),
    };

    let client = client(global)?;
    let mut action = client.create_expenses(expenses);
    if let Some(email) = &args.employee_email {
        action = action.employee_email(email);
    }

    let created = action.await.context("creating expenses")?;

    View::new(
        "expenses",
        vec![
            "TRANSACTION ID",
            "MERCHANT",
            "DATE",
            "AMOUNT CENTS",
            "CURRENCY",
        ],
        created
            .iter()
            .map(|transaction| {
                vec![
                    transaction.transaction_id.to_string(),
                    transaction.merchant.clone(),
                    transaction.created.to_string(),
                    transaction.amount_cents.to_string(),
                    transaction.currency.to_string(),
                ]
            })
            .collect(),
        Value::Array(created.iter().map(view::created_transaction).collect()),
    )
    .print(global.output)
}

/// The one-expense form. clap's `requires_all` guarantees the three
/// required flags arrive together.
fn inline_expense(args: &CreateExpensesArgs) -> Result<Expense> {
    let (Some(merchant), Some(date), Some(amount_cents)) =
        (&args.merchant, args.date, args.amount_cents)
    else {
        bail!("--merchant, --date and --amount-cents go together");
    };

    let mut expense = Expense::new(
        merchant,
        date,
        Money::new(amount_cents, args.currency.as_str()),
    );
    if let Some(category) = &args.category {
        expense = expense.category(category);
    }
    if let Some(tag) = &args.tag {
        expense = expense.tag(tag);
    }
    if let Some(comment) = &args.comment {
        expense = expense.comment(comment);
    }
    if let Some(id) = &args.external_id {
        expense = expense.external_id(id);
    }
    if let Some(id) = &args.report_id {
        expense = expense.report_id(id.as_str());
    }
    if let Some(id) = &args.policy_id {
        expense = expense.policy_id(id.as_str());
    }
    if let Some(billable) = args.billable {
        expense = expense.billable(billable);
    }
    if let Some(reimbursable) = args.reimbursable {
        expense = expense.reimbursable(reimbursable);
    }
    if let Some(rate_id) = &args.tax_rate_id {
        let mut tax = expensify::ExpenseTax::new(rate_id.as_str());
        if let Some(cents) = args.tax_amount_cents {
            tax = tax.amount_cents(cents);
        }
        expense = expense.tax(tax);
    }
    Ok(expense)
}

async fn report(args: CreateReportArgs, global: &GlobalArgs) -> Result<()> {
    let specs: Vec<ExpenseLineSpec> = read_json(&args.expenses)?;
    if specs.is_empty() {
        bail!("{} contains no expense lines", args.expenses);
    }
    let lines = specs
        .into_iter()
        .map(ExpenseLineSpec::build)
        .collect::<Result<Vec<_>>>()?;

    let client = client(global)?;
    let mut action = client.create_report(
        args.policy_id.as_str(),
        &args.employee_email,
        &args.title,
        lines,
    );
    for raw in &args.fields {
        let (name, value) = parse_pair(raw, "--field").unwrap_or_else(|err| usage_error(err));
        action = action.report_field(name, value);
    }

    let created = action.await.context("creating the report")?;

    View::new(
        "reports",
        vec!["REPORT ID", "NAME"],
        vec![vec![created.report_id.to_string(), created.name.clone()]],
        view::created_report(&created),
    )
    .print(global.output)
}

async fn expense_rule(args: CreateExpenseRuleArgs, global: &GlobalArgs) -> Result<()> {
    if args.tag.is_none() && args.default_billable.is_none() {
        usage_error("an expense rule needs --tag or --default-billable");
    }

    let client = client(global)?;
    let mut action = client.create_expense_rule(args.policy_id.as_str(), &args.employee_email);
    if let Some(tag) = &args.tag {
        action = action.tag(tag);
    }
    if let Some(billable) = args.default_billable {
        action = action.default_billable(billable);
    }
    action.await.context("creating the expense rule")?;

    // Expensify documents no response body, so there is no rule ID to
    // report back.
    View::acknowledgement(
        "expense rules",
        format!(
            "created a rule for {} on {}",
            args.employee_email, args.policy_id
        ),
    )
    .print(global.output)
}
