//! JSON projections of the library's response types.
//!
//! Those types are `Deserialize`-only, so `-o json` cannot re-serialize
//! them. Every shape the CLI emits is spelled out here instead — one place
//! to read, and `snake_case` like the input specs rather than the wire's
//! `camelCase`.

use expensify::{
    Category, CreatedPolicy, CreatedReport, CreatedTransaction, DomainCard, EmployeeUpdateOutcome,
    PolicyEmployee, PolicyRole, PolicySummary, PolicyTag, PolicyTags, ReimburseOutcome,
    ReportField, ReportFieldType, SkippedReport, TaxConfig,
};
use serde_json::{Value, json};

/// Round-trip an enum through its own `Serialize` impl to recover the word
/// Expensify uses, including the verbatim payload of an unmodelled variant.
fn wire_word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub fn role(role: &PolicyRole) -> String {
    wire_word(role)
}

pub fn plan(plan: &expensify::PolicyPlan) -> String {
    wire_word(plan)
}

/// `ReportFieldType` is read-only in the library, so it has no `Serialize`
/// to borrow.
pub fn report_field_type(field_type: &ReportFieldType) -> String {
    match field_type {
        ReportFieldType::Formula => "formula",
        ReportFieldType::Text => "text",
        ReportFieldType::Dropdown => "dropdown",
        ReportFieldType::Date => "date",
        ReportFieldType::Other(raw) => raw.as_str(),
        _ => "unknown",
    }
    .to_owned()
}

pub fn policy_summary(policy: &PolicySummary) -> Value {
    json!({
        "id": policy.id.as_str(),
        "name": policy.name,
        "owner": policy.owner,
        "role": role(&policy.role),
        "output_currency": policy.output_currency.as_str(),
        "plan": plan(&policy.plan),
    })
}

pub fn category(category: &Category) -> Value {
    json!({
        "name": category.name,
        "enabled": category.enabled,
        "gl_code": category.gl_code,
        "payroll_code": category.payroll_code,
        "comment_hint": category.comment_hint,
        "are_comments_required": category.are_comments_required,
        "max_expense_amount_cents": category.max_expense_amount_cents,
    })
}

pub fn report_field(field: &ReportField) -> Value {
    json!({
        "name": field.name,
        "type": report_field_type(&field.field_type),
        "values": field.values,
    })
}

pub fn tag(tag: &PolicyTag) -> Value {
    json!({
        "name": tag.name,
        "enabled": tag.enabled,
        "gl_code": tag.gl_code,
    })
}

/// Both shapes Expensify answers with are preserved; flattening them would
/// throw away the level names.
pub fn tags(tags: &PolicyTags) -> Value {
    match tags {
        PolicyTags::Flat(flat) => json!({
            "shape": "flat",
            "tags": flat.iter().map(tag).collect::<Vec<_>>(),
        }),
        PolicyTags::Levels(levels) => json!({
            "shape": "levels",
            "levels": levels
                .iter()
                .map(|level| json!({
                    "name": level.name,
                    "tags": level.tags.iter().map(tag).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }),
        _ => json!({ "shape": "unknown" }),
    }
}

pub fn tax(tax: &Option<TaxConfig>) -> Value {
    match tax {
        None => Value::Null,
        Some(config) => json!({
            "name": config.name,
            "default_rate_id": config.default.as_str(),
            "rates": config.rates.iter().map(|rate| json!({
                "name": rate.name,
                "rate": rate.rate,
                "rate_id": rate.rate_id.as_str(),
            })).collect::<Vec<_>>(),
        }),
    }
}

pub fn policy_employee(employee: &PolicyEmployee) -> Value {
    json!({
        "email": employee.email,
        "role": role(&employee.role),
        "submits_to": employee.submits_to,
        "employee_id": employee.employee_id,
        "custom_field_1": employee.custom_field_1,
        "custom_field_2": employee.custom_field_2,
    })
}

pub fn domain_card(card: &DomainCard) -> Value {
    json!({
        "bank": card.bank,
        "card_id": card.card_id,
        "card_name": card.card_name,
        "card_number": card.card_number,
        "email": card.email,
        "external_employee_id": card.external_employee_id,
        "created": card.created.map(|at| at.to_string()),
        "last_import": card.last_import.map(|at| at.to_string()),
        "last_import_result": card.last_import_result,
        "reimbursable": card.reimbursable,
        "scrape_min_date": card.scrape_min_date.map(|date| date.to_string()),
    })
}

pub fn created_policy(policy: &CreatedPolicy) -> Value {
    json!({ "policy_id": policy.policy_id.as_str(), "name": policy.name })
}

pub fn created_report(report: &CreatedReport) -> Value {
    json!({ "report_id": report.report_id.as_str(), "name": report.name })
}

pub fn created_transaction(transaction: &CreatedTransaction) -> Value {
    json!({
        "transaction_id": transaction.transaction_id.as_str(),
        "merchant": transaction.merchant,
        "date": transaction.created.to_string(),
        "amount_cents": transaction.amount_cents,
        "currency": transaction.currency.as_str(),
    })
}

fn skipped(report: &SkippedReport) -> Value {
    json!({ "report_id": report.report_id.as_str(), "reason": report.reason })
}

pub fn reimburse_outcome(outcome: &ReimburseOutcome) -> Value {
    json!({
        "updated": outcome.updated.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
        "skipped": outcome.skipped.iter().map(skipped).collect::<Vec<_>>(),
        "failed": outcome.failed.iter().map(skipped).collect::<Vec<_>>(),
    })
}

pub fn employee_outcome(outcome: &EmployeeUpdateOutcome) -> Value {
    let by_policy = |map: &std::collections::HashMap<expensify::PolicyId, Vec<String>>| {
        map.iter()
            .map(|(policy, emails)| (policy.as_str().to_owned(), json!(emails)))
            .collect::<serde_json::Map<_, _>>()
    };
    json!({
        "dry_run": outcome.dry_run,
        "updated_count": outcome.updated_count,
        "added": by_policy(&outcome.added),
        "removed": by_policy(&outcome.removed),
        "security_group_assignments": outcome.security_group_assignments,
        "skipped": outcome.skipped.iter().map(|employee| json!({
            "email": employee.email,
            "reason": employee.reason,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_render_as_wire_words_including_unmodelled_ones() {
        assert_eq!(role(&PolicyRole::Admin), "admin");
        assert_eq!(role(&PolicyRole::Other("copilot".to_owned())), "copilot");
    }

    #[test]
    fn a_category_renders_every_field_in_snake_case() {
        let rendered = category(&Category::new("Meals").gl_code("4000").require_comments());
        assert_eq!(rendered["name"], "Meals");
        assert_eq!(rendered["gl_code"], "4000");
        assert_eq!(rendered["are_comments_required"], true);
        assert_eq!(rendered["max_expense_amount_cents"], Value::Null);
    }

    #[test]
    fn unmodelled_report_field_types_keep_their_wire_word() {
        assert_eq!(report_field_type(&ReportFieldType::Formula), "formula");
        assert_eq!(
            report_field_type(&ReportFieldType::Other("currency".to_owned())),
            "currency"
        );
    }

    #[test]
    fn tag_shapes_are_labelled_rather_than_flattened() {
        let flat = tags(&PolicyTags::Flat(vec![PolicyTag::new("Enterprise")]));
        assert_eq!(flat["shape"], "flat");
        assert_eq!(flat["tags"][0]["name"], "Enterprise");
    }
}
