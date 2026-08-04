//! JSON shapes for the inputs that are too big for flags.
//!
//! The library's write types are builders with private fields, so these are
//! hand-written mirrors. They are `snake_case` throughout — one rule for
//! every input file, rather than the wire's `camelCase` for some types and
//! not others.

use anyhow::{Context, Result, bail};
use expensify::{
    Category, Employee, Expense, ExpenseLine, ExpenseTax, Money, PolicyRole, PolicyTag,
    ReportFieldDef, ReportFieldDefType, ReportFieldValue, TagLevel,
};
use serde::Deserialize;
use time::Date;
use time::macros::format_description;

pub fn parse_date(raw: &str) -> Result<Date, String> {
    Date::parse(raw, format_description!("[year]-[month]-[day]"))
        .map_err(|_| format!("`{raw}` is not a date; expected YYYY-MM-DD"))
}

fn date(raw: &str) -> Result<Date> {
    parse_date(raw).map_err(|err| anyhow::anyhow!(err))
}

fn enabled() -> bool {
    true
}

fn usd() -> String {
    "USD".to_owned()
}

/// Read a path, or stdin when it is `-`.
pub fn read_input(path: &str) -> Result<String> {
    if path == "-" {
        std::io::read_to_string(std::io::stdin().lock()).context("reading stdin")
    } else {
        std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
    }
}

/// Read a path as bytes, or stdin when it is `-`.
pub fn read_input_bytes(path: &str) -> Result<Vec<u8>> {
    if path == "-" {
        use std::io::Read as _;
        let mut buffer = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut buffer)
            .context("reading stdin")?;
        Ok(buffer)
    } else {
        std::fs::read(path).with_context(|| format!("reading {path}"))
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T> {
    let raw = read_input(path)?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {path}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpenseSpec {
    pub merchant: String,
    pub date: String,
    pub amount_cents: i64,
    #[serde(default = "usd")]
    pub currency: String,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub billable: Option<bool>,
    #[serde(default)]
    pub reimbursable: Option<bool>,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub policy_id: Option<String>,
    #[serde(default)]
    pub tax: Option<TaxSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxSpec {
    pub rate_id: String,
    #[serde(default)]
    pub amount_cents: Option<i64>,
}

impl ExpenseSpec {
    pub fn build(self) -> Result<Expense> {
        let mut expense = Expense::new(
            self.merchant,
            date(&self.date)?,
            Money::new(self.amount_cents, self.currency),
        );
        if let Some(id) = self.external_id {
            expense = expense.external_id(id);
        }
        if let Some(category) = self.category {
            expense = expense.category(category);
        }
        if let Some(tag) = self.tag {
            expense = expense.tag(tag);
        }
        if let Some(comment) = self.comment {
            expense = expense.comment(comment);
        }
        if let Some(billable) = self.billable {
            expense = expense.billable(billable);
        }
        if let Some(reimbursable) = self.reimbursable {
            expense = expense.reimbursable(reimbursable);
        }
        if let Some(id) = self.report_id {
            expense = expense.report_id(id);
        }
        if let Some(id) = self.policy_id {
            expense = expense.policy_id(id);
        }
        if let Some(tax) = self.tax {
            let mut applied = ExpenseTax::new(tax.rate_id);
            if let Some(cents) = tax.amount_cents {
                applied = applied.amount_cents(cents);
            }
            expense = expense.tax(applied);
        }
        Ok(expense)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpenseLineSpec {
    pub merchant: String,
    pub date: String,
    pub amount_cents: i64,
    #[serde(default = "usd")]
    pub currency: String,
}

impl ExpenseLineSpec {
    pub fn build(self) -> Result<ExpenseLine> {
        Ok(ExpenseLine::new(
            self.merchant,
            date(&self.date)?,
            Money::new(self.amount_cents, self.currency),
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategorySpec {
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub gl_code: Option<String>,
    #[serde(default)]
    pub payroll_code: Option<String>,
    #[serde(default)]
    pub comment_hint: Option<String>,
    #[serde(default)]
    pub require_comments: Option<bool>,
    #[serde(default)]
    pub max_expense_amount_cents: Option<i64>,
}

impl CategorySpec {
    pub fn build(self) -> Category {
        let mut category = Category::new(self.name);
        if !self.enabled {
            category = category.disabled();
        }
        if let Some(code) = self.gl_code {
            category = category.gl_code(code);
        }
        if let Some(code) = self.payroll_code {
            category = category.payroll_code(code);
        }
        if let Some(hint) = self.comment_hint {
            category = category.comment_hint(hint);
        }
        if self.require_comments == Some(true) {
            category = category.require_comments();
        }
        if let Some(cents) = self.max_expense_amount_cents {
            category = category.max_expense_amount_cents(cents);
        }
        category
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFieldSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: ReportFieldKind,
    #[serde(default)]
    pub values: Vec<ReportFieldValueSpec>,
    #[serde(default)]
    pub default_value: Option<String>,
}

/// The updater's three kinds. `formula` is read-only upstream and has no
/// spelling here, matching the library's `ReportFieldDefType`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFieldKind {
    Text,
    Dropdown,
    Date,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ReportFieldValueSpec {
    Name(String),
    Detailed {
        name: String,
        #[serde(default = "enabled")]
        enabled: bool,
        #[serde(default)]
        external_id: Option<String>,
    },
}

impl ReportFieldSpec {
    pub fn build(self) -> ReportFieldDef {
        let kind = match self.field_type {
            ReportFieldKind::Text => ReportFieldDefType::Text,
            ReportFieldKind::Dropdown => ReportFieldDefType::Dropdown,
            ReportFieldKind::Date => ReportFieldDefType::Date,
        };
        let mut field = ReportFieldDef::new(self.name, kind).values(
            self.values
                .into_iter()
                .map(|value| match value {
                    ReportFieldValueSpec::Name(name) => ReportFieldValue::new(name),
                    ReportFieldValueSpec::Detailed {
                        name,
                        enabled,
                        external_id,
                    } => {
                        let mut built = ReportFieldValue::new(name);
                        if !enabled {
                            built = built.disabled();
                        }
                        if let Some(id) = external_id {
                            built = built.external_id(id);
                        }
                        built
                    }
                })
                .collect::<Vec<_>>(),
        );
        if let Some(default) = self.default_value {
            field = field.default_value(default);
        }
        field
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagLevelSpec {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub required: bool,
    pub tags: Vec<TagSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagSpec {
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub gl_code: Option<String>,
}

impl TagLevelSpec {
    pub fn build(self) -> TagLevel {
        let tags = self.tags.into_iter().map(|tag| {
            let mut built = PolicyTag::new(tag.name);
            if !tag.enabled {
                built = built.disabled();
            }
            if let Some(code) = tag.gl_code {
                built = built.gl_code(code);
            }
            built
        });
        let mut level = TagLevel::new(tags.collect::<Vec<_>>());
        if let Some(name) = self.name {
            level = level.named(name);
        }
        if self.required {
            level = level.required();
        }
        level
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmployeeSpec {
    pub employee_email: String,
    pub manager_email: String,
    pub employee_id: String,
    pub policy_id: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub custom_field_1: Option<String>,
    #[serde(default)]
    pub custom_field_2: Option<String>,
    #[serde(default)]
    pub approval_limit: Option<i64>,
    #[serde(default)]
    pub over_limit_approver: Option<String>,
    #[serde(default)]
    pub worker_status: Option<String>,
    #[serde(default)]
    pub terminated: bool,
    #[serde(default)]
    pub domain_group_id: Option<String>,
    #[serde(default)]
    pub approves_to: Option<String>,
    #[serde(default)]
    pub role: Option<PolicyRole>,
    #[serde(default)]
    pub additional_policy_ids: Vec<String>,
    #[serde(default)]
    pub remove_from_unassigned_policies: bool,
    #[serde(default)]
    pub default_tags: Vec<String>,
}

impl EmployeeSpec {
    pub fn build(self) -> Employee {
        let mut employee = Employee::new(
            self.employee_email,
            self.manager_email,
            self.employee_id,
            self.policy_id,
        );
        if let Some(name) = self.first_name {
            employee = employee.first_name(name);
        }
        if let Some(name) = self.last_name {
            employee = employee.last_name(name);
        }
        if let Some(value) = self.custom_field_1 {
            employee = employee.custom_field_1(value);
        }
        if let Some(value) = self.custom_field_2 {
            employee = employee.custom_field_2(value);
        }
        if let Some(limit) = self.approval_limit {
            employee = employee.approval_limit(limit);
        }
        if let Some(email) = self.over_limit_approver {
            employee = employee.over_limit_approver(email);
        }
        if let Some(status) = self.worker_status {
            employee = employee.worker_status(status);
        }
        if self.terminated {
            employee = employee.terminated();
        }
        if let Some(id) = self.domain_group_id {
            employee = employee.domain_group_id(id);
        }
        if let Some(email) = self.approves_to {
            employee = employee.approves_to(email);
        }
        if let Some(role) = self.role {
            employee = employee.role(role);
        }
        if !self.additional_policy_ids.is_empty() {
            employee = employee.additional_policy_ids(self.additional_policy_ids);
        }
        if self.remove_from_unassigned_policies {
            employee = employee.remove_from_unassigned_policies();
        }
        if !self.default_tags.is_empty() {
            employee = employee.default_tags(self.default_tags);
        }
        employee
    }
}

/// `NAME=VALUE`, split on the first `=` so values may contain one.
pub fn parse_pair<'a>(raw: &'a str, flag: &str) -> Result<(&'a str, &'a str)> {
    match raw.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name, value)),
        _ => bail!("{flag} expects NAME=VALUE, got `{raw}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expense_needs_only_three_fields() {
        let spec: ExpenseSpec = serde_json::from_str(
            r#"{"merchant":"Cloud Hosting Inc","date":"2026-07-31","amount_cents":12900}"#,
        )
        .unwrap();
        assert_eq!(spec.currency, "USD");
        spec.build().unwrap();
    }

    #[test]
    fn a_misspelled_field_is_rejected_not_ignored() {
        let err = serde_json::from_str::<ExpenseSpec>(
            r#"{"merchant":"X","date":"2026-07-31","amount_cents":1,"catagory":"Meals"}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("catagory"), "{err}");
    }

    #[test]
    fn a_bad_date_is_reported_with_the_expected_shape() {
        let spec: ExpenseSpec =
            serde_json::from_str(r#"{"merchant":"X","date":"31/07/2026","amount_cents":1}"#)
                .unwrap();
        let err = spec.build().unwrap_err();
        assert!(err.to_string().contains("YYYY-MM-DD"), "{err}");
    }

    #[test]
    fn report_field_values_accept_both_spellings() {
        let spec: ReportFieldSpec = serde_json::from_str(
            r#"{"name":"Cost Center","type":"dropdown",
                "values":["Ops",{"name":"Eng","enabled":false,"external_id":"e1"}]}"#,
        )
        .unwrap();
        let built = spec.build();
        assert_eq!(built.values.len(), 2);
        assert!(built.values[0].enabled);
        assert!(!built.values[1].enabled);
        assert_eq!(built.values[1].external_id.as_deref(), Some("e1"));
    }

    #[test]
    fn pairs_split_on_the_first_equals() {
        assert_eq!(parse_pair("a=b=c", "--field").unwrap(), ("a", "b=c"));
        assert!(parse_pair("=b", "--field").is_err());
        assert!(parse_pair("ab", "--field").is_err());
    }
}
