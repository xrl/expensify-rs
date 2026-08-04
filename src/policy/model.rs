//! Policy data types shared between the getter (deserialized) and the
//! updater (serialized).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::types::{Currency, PolicyId, TaxRateId};

/// Getter samples always carry `enabled`, but the updater tables list it as
/// optional and both directions share these structs.
fn enabled_by_default() -> bool {
    true
}

/// A policy category. Read back by the Policy Getter; sent by the Policy
/// Updater (same wire shape both ways).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Category name, as shown in Expensify.
    pub name: String,
    /// Whether the category can be selected on new expenses. Absent on the
    /// wire means enabled.
    #[serde(default = "enabled_by_default")]
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
    /// Whether the tag can be selected on new expenses. Absent on the wire
    /// means enabled.
    #[serde(default = "enabled_by_default")]
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

/// One tag level as the Policy Getter returns it: a name plus the tags it
/// contains.
///
/// This is the read-side counterpart of [`TagLevel`](crate::TagLevel).
/// Expensify's getter sample carries only `name` and `tags`, so there is no
/// "required" flag here even though the updater has one.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PolicyTagLevel {
    /// Level name, e.g. `"Tags"`.
    #[serde(default)]
    pub name: Option<String>,
    /// Tags in this level.
    #[serde(default)]
    pub tags: Vec<PolicyTag>,
}

/// Tags as the Policy Getter returns them.
///
/// Expensify answers with one of two shapes and does not say which to
/// expect: a flat list of tags, or a list of tag *levels* each wrapping its
/// own tags. Both appear in the same page of Expensify's own documentation,
/// so both are modelled here rather than one being guessed at. Use
/// [`PolicyTags::tags`] when the level structure does not matter.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PolicyTags {
    /// `[{"name":"Enterprise","enabled":true,"glCode":""}]` — a
    /// single-level policy's tags, unwrapped.
    Flat(Vec<PolicyTag>),
    /// `[{"name":"Tags","tags":[...]}]` — one entry per tag level.
    Levels(Vec<PolicyTagLevel>),
}

impl PolicyTags {
    /// Every tag, flattened across levels.
    pub fn tags(&self) -> impl Iterator<Item = &PolicyTag> {
        let (flat, levels) = match self {
            Self::Flat(tags) => (Some(tags), None),
            Self::Levels(levels) => (None, Some(levels)),
        };
        flat.into_iter().flatten().chain(
            levels
                .into_iter()
                .flatten()
                .flat_map(|level| level.tags.iter()),
        )
    }
}

/// Hand-written rather than `#[serde(untagged)]`: the discriminator is the
/// presence of a `tags` key on the elements, and untagged would report a
/// genuine shape error as "did not match any variant".
impl<'de> Deserialize<'de> for PolicyTags {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
        let level_wrapped = raw.iter().any(|entry| entry.get("tags").is_some());

        if level_wrapped {
            raw.into_iter()
                .map(|entry| serde_json::from_value(entry).map_err(D::Error::custom))
                .collect::<Result<Vec<PolicyTagLevel>, _>>()
                .map(Self::Levels)
        } else {
            raw.into_iter()
                .map(|entry| serde_json::from_value(entry).map_err(D::Error::custom))
                .collect::<Result<Vec<PolicyTag>, _>>()
                .map(Self::Flat)
        }
    }
}

/// Kind of a report field, as the Policy Getter reports it.
///
/// Open: Expensify owns this vocabulary and adds to it without notice, so
/// an unrecognized value lands in [`ReportFieldType::Other`] instead of
/// failing the whole policy read. The updater accepts a strictly narrower
/// set — see [`ReportFieldDefType`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ReportFieldType {
    /// Computed server-side (e.g. the report title). Read-only: the updater
    /// rejects it, which is why [`ReportFieldDefType`] has no such variant.
    Formula,
    /// Free text.
    Text,
    /// Pick from [`ReportField::values`].
    Dropdown,
    /// Date picker.
    Date,
    /// A kind this crate does not model, verbatim from the wire.
    #[serde(untagged)]
    Other(String),
}

/// Kind of a report field the Policy Updater will accept.
///
/// Deliberately narrower than [`ReportFieldType`]: Expensify's updater
/// documents exactly these three and rejects `formula`, so that value is
/// not representable on a [`ReportFieldDef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFieldDefType {
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
    /// Field kind.
    #[serde(rename = "type")]
    pub field_type: ReportFieldDefType,
    /// Always serialized in object form; Expensify's "uniformly strings or
    /// uniformly objects" rule is therefore satisfied by construction.
    pub values: Vec<ReportFieldValue>,
    /// Pre-selected value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

impl ReportFieldDef {
    /// No values, no default.
    pub fn new(name: impl Into<String>, field_type: ReportFieldDefType) -> Self {
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
///
/// Open: Expensify owns this vocabulary, and one member with an unmodelled
/// role must not fail the whole read. [`PolicyRole::Other`] exists for the
/// read direction; sending one is a server-side rejection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PolicyRole {
    /// Submits expenses.
    User,
    /// Read-only oversight.
    Auditor,
    /// Full policy administration.
    Admin,
    /// A role this crate does not model, verbatim from the wire.
    #[serde(untagged)]
    Other(String),
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
///
/// Open: `free`, `control` and `personalPolicy` are all observed values
/// that the Policy Creator does not document, and one such policy must not
/// fail an entire [`Client::list_policies`](crate::Client::list_policies).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PolicyPlan {
    /// Team plan (the server default for new policies).
    Team,
    /// Corporate plan.
    Corporate,
    /// A plan this crate does not model, verbatim from the wire.
    #[serde(untagged)]
    Other(String),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Expensify's own single-policy sample shows tags in both shapes on
    /// the same page; either must decode.
    #[test]
    fn tags_decode_flat_and_level_wrapped() {
        let flat: PolicyTags = serde_json::from_value(
            json!([{ "glCode": "", "name": "Enterprise", "enabled": true }]),
        )
        .unwrap();
        assert!(matches!(&flat, PolicyTags::Flat(tags) if tags[0].name == "Enterprise"));

        let levels: PolicyTags = serde_json::from_value(json!([
            { "name": "Department", "tags": [{ "name": "Eng", "enabled": true }] },
            { "name": "Empty", "tags": [] }
        ]))
        .unwrap();
        match &levels {
            PolicyTags::Levels(levels) => {
                assert_eq!(levels[0].name.as_deref(), Some("Department"));
                assert!(levels[1].tags.is_empty());
            }
            other => panic!("expected levels, got {other:?}"),
        }

        // The reproducer from the review: a level with no tags and no
        // `enabled` key used to fail the whole `get_policies` call.
        let empty: PolicyTags =
            serde_json::from_value(json!([{ "name": "Tags", "tags": [] }])).unwrap();
        assert_eq!(empty.tags().count(), 0);
        assert_eq!(flat.tags().count(), 1);
        assert_eq!(levels.tags().count(), 1);
    }

    #[test]
    fn enabled_defaults_to_true_when_absent() {
        let category: Category = serde_json::from_value(json!({ "name": "Meals" })).unwrap();
        assert!(category.enabled);
        let tag: PolicyTag = serde_json::from_value(json!({ "name": "Core" })).unwrap();
        assert!(tag.enabled);
    }

    /// One policy on a plan this crate does not model must not fail the
    /// whole list.
    #[test]
    fn unknown_enum_values_round_trip_verbatim() {
        for raw in ["free", "control", "personalPolicy"] {
            let plan: PolicyPlan = serde_json::from_value(json!(raw)).unwrap();
            assert_eq!(plan, PolicyPlan::Other(raw.to_owned()));
            assert_eq!(serde_json::to_value(&plan).unwrap(), json!(raw));
        }
        assert_eq!(
            serde_json::from_value::<PolicyRole>(json!("copilot")).unwrap(),
            PolicyRole::Other("copilot".to_owned())
        );
        assert_eq!(
            serde_json::from_value::<ReportFieldType>(json!("currency")).unwrap(),
            ReportFieldType::Other("currency".to_owned())
        );
        // Known values still map to their named variants.
        assert_eq!(
            serde_json::from_value::<PolicyPlan>(json!("team")).unwrap(),
            PolicyPlan::Team
        );
        assert_eq!(
            serde_json::from_value::<ReportFieldType>(json!("formula")).unwrap(),
            ReportFieldType::Formula
        );
    }
}
