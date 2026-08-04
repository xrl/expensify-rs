//! Policy data types shared between the getter (deserialized) and the
//! updater (serialized).

use serde::{Deserialize, Serialize};

use crate::types::{Currency, PolicyId, TaxRateId};

/// A policy category. Read back by the Policy Getter; sent by the Policy
/// Updater (same wire shape both ways).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Category name, as shown in Expensify.
    pub name: String,
    /// Whether the category can be selected on new expenses.
    pub enabled: bool,
    /// General-ledger code for accounting exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gl_code: Option<String>,
    /// Payroll code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payroll_code: Option<String>,
    /// Whether expenses in this category must carry a comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub are_comments_required: Option<bool>,
    /// Placeholder text shown in the comment box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_hint: Option<String>,
    /// Integer cents (`maxExpenseAmount` on the wire).
    #[serde(
        rename = "maxExpenseAmount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_expense_amount_cents: Option<i64>,
}

impl Category {
    /// Enabled by default.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            gl_code: None,
            payroll_code: None,
            are_comments_required: None,
            comment_hint: None,
            max_expense_amount_cents: None,
        }
    }

    /// Keep the category but hide it from expense entry.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the GL code.
    pub fn gl_code(mut self, code: impl Into<String>) -> Self {
        self.gl_code = Some(code.into());
        self
    }

    /// Set the payroll code.
    pub fn payroll_code(mut self, code: impl Into<String>) -> Self {
        self.payroll_code = Some(code.into());
        self
    }

    /// Require a comment on expenses in this category.
    pub fn require_comments(mut self) -> Self {
        self.are_comments_required = Some(true);
        self
    }

    /// Hint text for the comment box.
    pub fn comment_hint(mut self, hint: impl Into<String>) -> Self {
        self.comment_hint = Some(hint.into());
        self
    }

    /// Cap in integer cents.
    pub fn max_expense_amount_cents(mut self, cents: i64) -> Self {
        self.max_expense_amount_cents = Some(cents);
        self
    }
}

/// A tag within one tag level. Same shape read and written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTag {
    /// Tag name.
    pub name: String,
    /// Whether the tag can be selected on new expenses.
    pub enabled: bool,
    /// General-ledger code for accounting exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gl_code: Option<String>,
}

impl PolicyTag {
    /// Enabled by default.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            gl_code: None,
        }
    }

    /// Keep the tag but hide it from expense entry.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the GL code.
    pub fn gl_code(mut self, code: impl Into<String>) -> Self {
        self.gl_code = Some(code.into());
        self
    }
}

/// Kind of a policy report field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFieldType {
    /// Read-only: appears in getter output (e.g. the report title) but is
    /// not creatable via the updater (server rejects it).
    Formula,
    /// Free text.
    Text,
    /// Pick from [`ReportFieldDef::values`].
    Dropdown,
    /// Date picker.
    Date,
}

/// Report field as returned by the Policy Getter.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportField {
    /// Field label.
    pub name: String,
    /// Field kind.
    #[serde(rename = "type")]
    pub field_type: ReportFieldType,
    /// Dropdown options, empty for other kinds.
    #[serde(default)]
    pub values: Vec<String>,
}

/// Report field definition for the Policy Updater.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportFieldDef {
    /// Field label.
    pub name: String,
    /// Field kind. [`ReportFieldType::Formula`] is rejected by the server.
    #[serde(rename = "type")]
    pub field_type: ReportFieldType,
    /// Always serialized in object form; Expensify's "uniformly strings or
    /// uniformly objects" rule is therefore satisfied by construction.
    pub values: Vec<ReportFieldValue>,
    /// Pre-selected value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

impl ReportFieldDef {
    /// No values, no default.
    pub fn new(name: impl Into<String>, field_type: ReportFieldType) -> Self {
        Self {
            name: name.into(),
            field_type,
            values: Vec::new(),
            default_value: None,
        }
    }

    /// Replace the dropdown options.
    pub fn values<I>(mut self, values: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<ReportFieldValue>,
    {
        self.values = values.into_iter().map(Into::into).collect();
        self
    }

    /// Pre-select a value.
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }
}

/// A dropdown value for a report field.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportFieldValue {
    /// Displayed value.
    pub name: String,
    /// Whether the option is selectable.
    pub enabled: bool,
    /// Caller-side identifier echoed back on export.
    #[serde(rename = "externalID", skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

impl ReportFieldValue {
    /// Enabled by default.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            external_id: None,
        }
    }

    /// Keep the option but hide it.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Attach an external identifier.
    pub fn external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }
}

impl From<&str> for ReportFieldValue {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for ReportFieldValue {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

/// Tax configuration of a policy (Policy Getter, `fields: ["tax"]`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxConfig {
    /// Display name of the tax scheme.
    pub name: String,
    /// `rateID` of the default rate.
    pub default: TaxRateId,
    /// Every configured rate.
    pub rates: Vec<TaxRate>,
}

/// One tax rate within a [`TaxConfig`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxRate {
    /// Display name.
    pub name: String,
    /// Percentage, e.g. `20.0`.
    pub rate: f64,
    /// Pass to [`ExpenseTax::new`](crate::ExpenseTax::new).
    #[serde(rename = "rateID")]
    pub rate_id: TaxRateId,
}

/// A member's role on a policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyRole {
    /// Submits expenses.
    User,
    /// Read-only oversight.
    Auditor,
    /// Full policy administration.
    Admin,
}

/// Policy member (Policy Getter, `fields: ["employees"]`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyEmployee {
    /// Login email.
    pub email: String,
    /// Role on this policy.
    pub role: PolicyRole,
    /// Email of the approver this member submits to.
    #[serde(default)]
    pub submits_to: Option<String>,
    /// External employee number.
    #[serde(rename = "employeeID", default)]
    pub employee_id: Option<String>,
    /// Custom Field 1 (auto-filled from `employee_id` unless set).
    #[serde(default)]
    pub custom_field_1: Option<String>,
    /// Custom Field 2.
    #[serde(default)]
    pub custom_field_2: Option<String>,
}

/// `type` on the wire; called "plan" here to avoid a third meaning of
/// "type".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyPlan {
    /// Team plan (the server default for new policies).
    Team,
    /// Corporate plan.
    Corporate,
}

/// One entry from the Policy List Getter.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicySummary {
    /// Policy identifier.
    pub id: PolicyId,
    /// Policy name.
    pub name: String,
    /// Email of the policy owner.
    pub owner: String,
    /// The credential owner's role on this policy.
    pub role: PolicyRole,
    /// Currency policy amounts are reported in.
    pub output_currency: Currency,
    /// Subscription plan.
    #[serde(rename = "type")]
    pub plan: PolicyPlan,
}
