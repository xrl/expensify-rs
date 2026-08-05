use bytes::Bytes;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::policy::model::{Category, PolicyTag, ReportFieldDef};
use crate::types::PolicyId;
use crate::wire;

/// Merge-vs-replace for a Policy Updater section. `ReplaceAll` deletes
/// everything not in the submitted list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateMode {
    Merge,
    ReplaceAll,
}

/// Category section of a Policy Updater request.
#[derive(Clone, Debug)]
pub struct CategoriesUpdate {
    pub(crate) mode: UpdateMode,
    pub(crate) data: Vec<Category>,
}

impl CategoriesUpdate {
    /// Update/add the listed categories, keep the rest (`action: "merge"`).
    pub fn merge<I: IntoIterator<Item = Category>>(categories: I) -> Self {
        Self {
            mode: UpdateMode::Merge,
            data: categories.into_iter().collect(),
        }
    }

    /// Replace the entire category list (`action: "replace"`). Destructive:
    /// categories absent from `categories` are deleted.
    pub fn replace_all<I: IntoIterator<Item = Category>>(categories: I) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            data: categories.into_iter().collect(),
        }
    }
}

/// Report-field section of a Policy Updater request.
#[derive(Clone, Debug)]
pub struct ReportFieldsUpdate {
    pub(crate) mode: UpdateMode,
    pub(crate) data: Vec<ReportFieldDef>,
}

impl ReportFieldsUpdate {
    /// Update/add the listed fields, keep the rest.
    pub fn merge<I: IntoIterator<Item = ReportFieldDef>>(fields: I) -> Self {
        Self {
            mode: UpdateMode::Merge,
            data: fields.into_iter().collect(),
        }
    }

    /// Replace the entire report-field list. Destructive.
    pub fn replace_all<I: IntoIterator<Item = ReportFieldDef>>(fields: I) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            data: fields.into_iter().collect(),
        }
    }
}

/// One tag level for the inline tag source. Independent levels only —
/// dependent (cascading) levels require the CSV source.
#[derive(Clone, Debug)]
pub struct TagLevel {
    /// Level name; only meaningful for multi-level policies.
    pub name: Option<String>,
    /// Whether a tag from this level is required on each expense.
    pub required: bool,
    /// The tags in this level.
    pub tags: Vec<PolicyTag>,
}

impl TagLevel {
    /// Unnamed and optional by default.
    pub fn new<I: IntoIterator<Item = PolicyTag>>(tags: I) -> Self {
        Self {
            name: None,
            required: false,
            tags: tags.into_iter().collect(),
        }
    }

    /// Name the level.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Require a tag from this level on every expense.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// Layout description for CSV/TSV tag uploads. The two constructors
/// mirror Expensify's rule that `setRequired` is a single boolean for
/// dependent levels but an array (one per level) for independent levels —
/// the wrong pairing is unrepresentable.
#[derive(Clone, Debug)]
pub struct TagCsvConfig {
    pub(crate) dependent: bool,
    pub(crate) set_required: Vec<bool>,
    pub(crate) gl_codes: bool,
    pub(crate) header_row: bool,
    pub(crate) tsv: bool,
}

impl TagCsvConfig {
    /// Dependent (cascading) levels: one `set_required` for the whole
    /// hierarchy.
    pub fn dependent(set_required: bool) -> Self {
        Self {
            dependent: true,
            set_required: vec![set_required],
            gl_codes: false,
            header_row: false,
            tsv: false,
        }
    }

    /// Independent levels: `set_required` per level, in column order.
    pub fn independent<I: IntoIterator<Item = bool>>(set_required: I) -> Self {
        Self {
            dependent: false,
            set_required: set_required.into_iter().collect(),
            gl_codes: false,
            header_row: false,
            tsv: false,
        }
    }

    /// Each tag column is followed by a GL-code column.
    pub fn with_gl_codes(mut self) -> Self {
        self.gl_codes = true;
        self
    }

    /// First row holds level names.
    pub fn with_header_row(mut self) -> Self {
        self.header_row = true;
        self
    }

    /// Tab-separated rather than comma-separated.
    pub fn tsv(mut self) -> Self {
        self.tsv = true;
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TagsSource {
    Inline(Vec<TagLevel>),
    Csv { data: Bytes, config: TagCsvConfig },
}

/// Tag section of a Policy Updater request.
///
/// Replace-only, unlike [`CategoriesUpdate`] and [`ReportFieldsUpdate`], and
/// now confirmed to be the only honest spelling. Sending one tag with
/// `action: "merge"` against a policy holding two others **deleted both**,
/// and answered `{"responseCode":200}` with no warning (observed live
/// 2026-08-04). A `merge_*` constructor would therefore be a `replace_all_*`
/// under a name that promises the opposite, which is precisely what this
/// crate's naming exists to prevent.
#[derive(Clone, Debug)]
pub struct TagsUpdate {
    pub(crate) mode: UpdateMode,
    pub(crate) source: TagsSource,
}

impl TagsUpdate {
    /// Replace the entire tag list from inline levels. Destructive.
    /// Independent levels only; the inline form has no dependency knob.
    pub fn replace_all_inline<I: IntoIterator<Item = TagLevel>>(levels: I) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            source: TagsSource::Inline(levels.into_iter().collect()),
        }
    }

    /// Replace the entire tag list from a CSV/TSV upload. Destructive.
    ///
    /// The data rides in the separate `file` form field; non-UTF-8 bytes are
    /// replaced, since that field is urlencoded text.
    pub fn replace_all_csv(data: impl Into<Bytes>, config: TagCsvConfig) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            source: TagsSource::Csv {
                data: data.into(),
                config,
            },
        }
    }
}

/// Policy Updater (`type: "update"`, `inputSettings.type: "policy"`).
/// Sections are independent; set any subset. Awaiting with no section set
/// is a no-op request the server accepts.
///
/// Requires policy-admin credentials (server-enforced).
#[must_use = "actions do nothing until awaited"]
pub struct UpdatePolicyAction {
    pub(crate) client: Client,
    pub(crate) policy_ids: Vec<PolicyId>,
    pub(crate) categories: Option<CategoriesUpdate>,
    pub(crate) report_fields: Option<ReportFieldsUpdate>,
    pub(crate) tags: Option<TagsUpdate>,
}

impl UpdatePolicyAction {
    pub(crate) fn new(client: Client, policy_ids: Vec<PolicyId>) -> Self {
        Self {
            client,
            policy_ids,
            categories: None,
            report_fields: None,
            tags: None,
        }
    }

    /// Set the categories section.
    pub fn categories(mut self, update: CategoriesUpdate) -> Self {
        self.categories = Some(update);
        self
    }

    /// Set the report-fields section.
    pub fn report_fields(mut self, update: ReportFieldsUpdate) -> Self {
        self.report_fields = Some(update);
        self
    }

    /// Set the tags section.
    pub fn tags(mut self, update: TagsUpdate) -> Self {
        self.tags = Some(update);
        self
    }
}

impl IntoFuture for UpdatePolicyAction {
    type Output = Result<(), Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if self.policy_ids.is_empty() {
                return Err(Error::InvalidRequest(
                    "update_policies needs at least one policy ID; \
                     an empty policyIDList is a documented 410"
                        .to_owned(),
                ));
            }
            let request = wire::update_policy(&self);
            self.client.send(request).await?;
            Ok(())
        })
    }
}
