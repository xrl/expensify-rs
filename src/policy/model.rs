//! Policy data types shared between the getter (deserialized) and the
//! updater (serialized).

use serde::{Deserialize, Serialize};

use crate::types::{Currency, PolicyId, TaxRateId};

/// A policy category. Read back by the Policy Getter; sent by the Policy
/// Updater (same wire shape both ways).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub enabled: bool,
    pub gl_code: Option<String>,
    pub payroll_code: Option<String>,
    pub are_comments_required: Option<bool>,
    pub comment_hint: Option<String>,
    /// Integer cents (`maxExpenseAmount` on the wire).
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

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn gl_code(mut self, code: impl Into<String>) -> Self {
        self.gl_code = Some(code.into());
        self
    }

    pub fn payroll_code(mut self, code: impl Into<String>) -> Self {
        self.payroll_code = Some(code.into());
        self
    }

    pub fn require_comments(mut self) -> Self {
        self.are_comments_required = Some(true);
        self
    }

    pub fn comment_hint(mut self, hint: impl Into<String>) -> Self {
        self.comment_hint = Some(hint.into());
        self
    }

    pub fn max_expense_amount_cents(mut self, cents: i64) -> Self {
        self.max_expense_amount_cents = Some(cents);
        self
    }
}

/// A tag within one tag level. Same shape read and written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyTag {
    pub name: String,
    pub enabled: bool,
    pub gl_code: Option<String>,
}

impl PolicyTag {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), enabled: true, gl_code: None }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn gl_code(mut self, code: impl Into<String>) -> Self {
        self.gl_code = Some(code.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFieldType {
    /// Read-only: appears in getter output (e.g. the report title) but is
    /// not creatable via the updater (server rejects it).
    Formula,
    Text,
    Dropdown,
    Date,
}

/// Report field as returned by the Policy Getter.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ReportField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: ReportFieldType,
    pub values: Vec<String>,
}

/// Report field definition for the Policy Updater.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportFieldDef {
    pub name: String,
    pub field_type: ReportFieldType,
    /// Always serialized in object form; Expensify's "uniformly strings or
    /// uniformly objects" rule is therefore satisfied by construction.
    pub values: Vec<ReportFieldValue>,
    pub default_value: Option<String>,
}

impl ReportFieldDef {
    pub fn new(name: impl Into<String>, field_type: ReportFieldType) -> Self {
        Self {
            name: name.into(),
            field_type,
            values: Vec::new(),
            default_value: None,
        }
    }

    pub fn values<I>(mut self, values: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<ReportFieldValue>,
    {
        self.values = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }
}

/// A dropdown value for a report field.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportFieldValue {
    pub name: String,
    pub enabled: bool,
    pub external_id: Option<String>,
}

impl ReportFieldValue {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), enabled: true, external_id: None }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

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
pub struct TaxConfig {
    pub name: String,
    /// `rateID` of the default rate.
    pub default: TaxRateId,
    pub rates: Vec<TaxRate>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TaxRate {
    pub name: String,
    /// Percentage, e.g. `20.0`.
    pub rate: f64,
    pub rate_id: TaxRateId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyRole {
    User,
    Auditor,
    Admin,
}

/// Policy member (Policy Getter, `fields: ["employees"]`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PolicyEmployee {
    pub email: String,
    pub role: PolicyRole,
    pub submits_to: Option<String>,
    pub employee_id: Option<String>,
    pub custom_field_1: Option<String>,
    pub custom_field_2: Option<String>,
}

/// `type` on the wire; called "plan" here to avoid a third meaning of
/// "type".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyPlan {
    Team,
    Corporate,
}

/// One entry from the Policy List Getter.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PolicySummary {
    pub id: PolicyId,
    pub name: String,
    pub owner: String,
    /// The credential owner's role on this policy.
    pub role: PolicyRole,
    pub output_currency: Currency,
    pub plan: PolicyPlan,
}
