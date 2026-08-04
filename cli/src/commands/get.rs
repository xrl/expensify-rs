//! `expensify get` — the read side.
//!
//! `get policy` is where the library's field typestate meets runtime flags;
//! see the `stage_*` ladder below.

use anyhow::{Context, Result, bail};
use expensify::{
    Category, FetchState, Fetched, GetPoliciesAction, GetPoliciesBuilder, NotFetched, Omitted,
    Payload, PolicyEmployee, PolicyId, PolicyTags, ReportField, TaxConfig,
};
use serde_json::{Value, json};

use crate::cli::{
    GetCardsArgs, GetCommand, GetPoliciesArgs, GetPolicyArgs, GlobalArgs, PolicySections,
};
use crate::commands::client;
use crate::output::{OutputFormat, View, cell_opt, render_table};
use crate::view;

pub async fn run(command: GetCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        GetCommand::Policies(args) => policies(args, global).await,
        GetCommand::Policy(args) => policy(args, global).await,
        GetCommand::Cards(args) => cards(args, global).await,
    }
}

async fn policies(args: GetPoliciesArgs, global: &GlobalArgs) -> Result<()> {
    let client = client(global)?;
    let mut action = client.list_policies();
    if args.admin_only {
        action = action.admin_only();
    }
    if let Some(email) = &args.on_behalf_of {
        action = action.on_behalf_of(email);
    }

    let policies = action.await.context("listing policies")?;

    let wide = global.output.is_wide();
    let headers = if wide {
        vec!["ID", "NAME", "ROLE", "OWNER", "CURRENCY", "PLAN"]
    } else {
        vec!["ID", "NAME", "ROLE"]
    };
    let rows = policies
        .iter()
        .map(|policy| {
            let mut row = vec![
                policy.id.to_string(),
                policy.name.clone(),
                view::role(&policy.role),
            ];
            if wide {
                row.push(policy.owner.clone());
                row.push(policy.output_currency.to_string());
                row.push(view::plan(&policy.plan));
            }
            row
        })
        .collect();
    let json = Value::Array(policies.iter().map(view::policy_summary).collect());

    View::new("policies", headers, rows, json).print(global.output)
}

async fn cards(args: GetCardsArgs, global: &GlobalArgs) -> Result<()> {
    let client = client(global)?;
    let cards = client
        .domain(&args.domain)
        .card_list()
        .await
        .with_context(|| format!("listing cards on {}", args.domain))?;

    let wide = global.output.is_wide();
    let headers = if wide {
        vec![
            "CARD ID",
            "NAME",
            "NUMBER",
            "EMAIL",
            "BANK",
            "REIMBURSABLE",
            "LAST IMPORT",
            "LAST RESULT",
        ]
    } else {
        vec!["CARD ID", "NAME", "NUMBER", "EMAIL"]
    };
    let rows = cards
        .iter()
        .map(|card| {
            let mut row = vec![
                card.card_id.to_string(),
                card.card_name.clone(),
                card.card_number.clone(),
                card.email.clone(),
            ];
            if wide {
                row.push(card.bank.clone());
                row.push(card.reimbursable.to_string());
                row.push(cell_opt(card.last_import.as_ref()));
                row.push(cell_opt(card.last_import_result));
            }
            row
        })
        .collect();
    let json = Value::Array(cards.iter().map(view::domain_card).collect());

    View::new("cards", headers, rows, json).print(global.output)
}

// ---- get policy: runtime flags over a compile-time typestate ---------

/// A policy once its typestate has been read back into `Option`s. `None` is
/// "not requested" — the library's `NotFetched`, which cannot be inspected
/// generically.
struct PolicyView {
    id: PolicyId,
    categories: Option<Vec<Category>>,
    report_fields: Option<Vec<ReportField>>,
    tags: Option<PolicyTags>,
    tax: Option<Option<TaxConfig>>,
    employees: Option<Vec<PolicyEmployee>>,
}

/// The inverse of [`FetchState::Wrap`], which the library does not provide:
/// `Wrap` maps a payload into a slot, and nothing maps a slot back out. It
/// is implementable outside the library only because both states and the
/// GAT are public.
trait Slot: FetchState {
    fn read<T: Payload>(slot: Self::Wrap<T>) -> Option<T>;
}

impl Slot for Fetched {
    fn read<T: Payload>(slot: T) -> Option<T> {
        Some(slot)
    }
}

impl Slot for Omitted {
    fn read<T: Payload>(_slot: NotFetched) -> Option<T> {
        None
    }
}

/// Five stages, one per type parameter, each deciding its own flag and
/// handing the next stage a more-`Fetched` action. Two branches per stage
/// rather than one match over 32 states; the compiler still generates the
/// 32 leaves.
async fn stage_categories(
    builder: GetPoliciesBuilder,
    want: PolicySections,
) -> Result<Vec<PolicyView>> {
    // `GetPoliciesBuilder` cannot produce an all-`Omitted` action — the
    // library requires at least one selection — so the first requested
    // section anchors the chain.
    if want.with_categories {
        stage_report_fields(builder.with_categories(), want).await
    } else if want.with_report_fields {
        stage_tags(builder.with_report_fields(), want).await
    } else if want.with_tags {
        stage_tax(builder.with_tags(), want).await
    } else if want.with_tax {
        stage_employees(builder.with_tax(), want).await
    } else if want.with_employees {
        finish(builder.with_employees()).await
    } else {
        // clap's required group already refuses this.
        bail!("select at least one --with-* section")
    }
}

async fn stage_report_fields<C: Slot>(
    action: GetPoliciesAction<C, Omitted, Omitted, Omitted, Omitted>,
    want: PolicySections,
) -> Result<Vec<PolicyView>> {
    if want.with_report_fields {
        stage_tags(action.with_report_fields(), want).await
    } else {
        stage_tags(action, want).await
    }
}

async fn stage_tags<C: Slot, F: Slot>(
    action: GetPoliciesAction<C, F, Omitted, Omitted, Omitted>,
    want: PolicySections,
) -> Result<Vec<PolicyView>> {
    if want.with_tags {
        stage_tax(action.with_tags(), want).await
    } else {
        stage_tax(action, want).await
    }
}

async fn stage_tax<C: Slot, F: Slot, T: Slot>(
    action: GetPoliciesAction<C, F, T, Omitted, Omitted>,
    want: PolicySections,
) -> Result<Vec<PolicyView>> {
    if want.with_tax {
        stage_employees(action.with_tax(), want).await
    } else {
        stage_employees(action, want).await
    }
}

async fn stage_employees<C: Slot, F: Slot, T: Slot, X: Slot>(
    action: GetPoliciesAction<C, F, T, X, Omitted>,
    want: PolicySections,
) -> Result<Vec<PolicyView>> {
    if want.with_employees {
        finish(action.with_employees()).await
    } else {
        finish(action).await
    }
}

async fn finish<C: Slot, F: Slot, T: Slot, X: Slot, E: Slot>(
    action: GetPoliciesAction<C, F, T, X, E>,
) -> Result<Vec<PolicyView>> {
    let policies = action.await.context("reading policies")?;
    let mut views: Vec<_> = policies
        .into_iter()
        .map(|(id, policy)| PolicyView {
            id,
            categories: C::read(policy.categories),
            report_fields: F::read(policy.report_fields),
            tags: T::read(policy.tags),
            tax: X::read(policy.tax),
            employees: E::read(policy.employees),
        })
        .collect();
    // The response is a map; sort so output is stable between runs.
    views.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    Ok(views)
}

async fn policy(args: GetPolicyArgs, global: &GlobalArgs) -> Result<()> {
    let client = client(global)?;

    let mut builder = client.get_policies(args.policy_ids.iter().map(String::as_str));
    if let Some(email) = &args.on_behalf_of {
        builder = builder.on_behalf_of(email);
    }

    let policies = stage_categories(builder, args.sections).await?;

    match global.output {
        OutputFormat::Json => {
            let json = Value::Array(policies.iter().map(policy_json).collect());
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => print_policy_tables(&policies),
    }
    Ok(())
}

/// Sections that were not requested are absent, not null: the caller knows
/// which flags they passed, and a null would read as "empty".
fn policy_json(policy: &PolicyView) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(policy.id.as_str()));
    if let Some(categories) = &policy.categories {
        map.insert(
            "categories".into(),
            Value::Array(categories.iter().map(view::category).collect()),
        );
    }
    if let Some(fields) = &policy.report_fields {
        map.insert(
            "report_fields".into(),
            Value::Array(fields.iter().map(view::report_field).collect()),
        );
    }
    if let Some(tags) = &policy.tags {
        map.insert("tags".into(), view::tags(tags));
    }
    if let Some(tax) = &policy.tax {
        map.insert("tax".into(), view::tax(tax));
    }
    if let Some(employees) = &policy.employees {
        map.insert(
            "employees".into(),
            Value::Array(employees.iter().map(view::policy_employee).collect()),
        );
    }
    Value::Object(map)
}

fn print_policy_tables(policies: &[PolicyView]) {
    for policy in policies {
        if let Some(categories) = &policy.categories {
            let rows = categories
                .iter()
                .map(|category| {
                    vec![
                        category.name.clone(),
                        category.enabled.to_string(),
                        cell_opt(category.gl_code.as_ref()),
                        cell_opt(category.payroll_code.as_ref()),
                        cell_opt(category.max_expense_amount_cents),
                    ]
                })
                .collect::<Vec<_>>();
            section(
                &policy.id,
                "categories",
                &["NAME", "ENABLED", "GL CODE", "PAYROLL CODE", "MAX CENTS"],
                &rows,
            );
        }
        if let Some(fields) = &policy.report_fields {
            let rows = fields
                .iter()
                .map(|field| {
                    vec![
                        field.name.clone(),
                        view::report_field_type(&field.field_type),
                        field.values.join(", "),
                    ]
                })
                .collect::<Vec<_>>();
            section(
                &policy.id,
                "report fields",
                &["NAME", "TYPE", "VALUES"],
                &rows,
            );
        }
        if let Some(tags) = &policy.tags {
            let rows = match tags {
                PolicyTags::Flat(flat) => flat
                    .iter()
                    .map(|tag| {
                        vec![
                            String::new(),
                            tag.name.clone(),
                            tag.enabled.to_string(),
                            cell_opt(tag.gl_code.as_ref()),
                        ]
                    })
                    .collect::<Vec<_>>(),
                PolicyTags::Levels(levels) => levels
                    .iter()
                    .flat_map(|level| {
                        let name = level.name.clone().unwrap_or_default();
                        level.tags.iter().map(move |tag| {
                            vec![
                                name.clone(),
                                tag.name.clone(),
                                tag.enabled.to_string(),
                                cell_opt(tag.gl_code.as_ref()),
                            ]
                        })
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            section(
                &policy.id,
                "tags",
                &["LEVEL", "NAME", "ENABLED", "GL CODE"],
                &rows,
            );
        }
        if let Some(tax) = &policy.tax {
            let rows = match tax {
                None => Vec::new(),
                Some(config) => config
                    .rates
                    .iter()
                    .map(|rate| {
                        vec![
                            rate.name.clone(),
                            format!("{}%", rate.rate),
                            rate.rate_id.to_string(),
                            (rate.rate_id == config.default).to_string(),
                        ]
                    })
                    .collect(),
            };
            section(
                &policy.id,
                "tax rates",
                &["NAME", "RATE", "RATE ID", "DEFAULT"],
                &rows,
            );
        }
        if let Some(employees) = &policy.employees {
            let rows = employees
                .iter()
                .map(|employee| {
                    vec![
                        employee.email.clone(),
                        view::role(&employee.role),
                        cell_opt(employee.submits_to.as_ref()),
                        cell_opt(employee.employee_id.as_ref()),
                    ]
                })
                .collect::<Vec<_>>();
            section(
                &policy.id,
                "employees",
                &["EMAIL", "ROLE", "SUBMITS TO", "EMPLOYEE ID"],
                &rows,
            );
        }
    }
}

fn section(policy: &PolicyId, name: &str, headers: &[&str], rows: &[Vec<String>]) {
    println!("POLICY {policy} — {}", name.to_uppercase());
    if rows.is_empty() {
        println!("(none)");
    } else {
        println!("{}", render_table(headers, rows));
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use expensify::{PolicyTag, TaxRate};

    fn view_with_sections() -> PolicyView {
        PolicyView {
            id: PolicyId::new("P1"),
            categories: Some(vec![Category::new("Meals")]),
            report_fields: None,
            tags: Some(PolicyTags::Flat(vec![PolicyTag::new("Core")])),
            tax: Some(Some(TaxConfig {
                name: "VAT".into(),
                default: "id_A".into(),
                rates: vec![TaxRate {
                    name: "Standard".into(),
                    rate: 20.0,
                    rate_id: "id_A".into(),
                }],
            })),
            employees: None,
        }
    }

    /// The point of the whole typestate: a section that was not requested
    /// must not appear as an empty one.
    #[test]
    fn json_omits_unrequested_sections() {
        let rendered = policy_json(&view_with_sections());
        assert!(rendered.get("categories").is_some());
        assert!(rendered.get("tax").is_some());
        assert!(rendered.get("report_fields").is_none());
        assert!(rendered.get("employees").is_none());
    }

    #[test]
    fn a_policy_with_no_tax_configuration_renders_null_not_absent() {
        let mut policy = view_with_sections();
        policy.tax = Some(None);
        assert_eq!(policy_json(&policy)["tax"], Value::Null);
    }
}
