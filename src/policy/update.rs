use bytes::Bytes;

use crate::client::Client;
use crate::error::Error;
use crate::policy::model::{Category, PolicyTag, ReportFieldDef};
use crate::types::PolicyId;
use crate::BoxFuture;

/// Merge-vs-replace for a Policy Updater section. `ReplaceAll` deletes
/// everything not in the submitted list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateMode {
    Merge,
    ReplaceAll,
}

#[derive(Clone, Debug)]
pub struct CategoriesUpdate {
    mode: UpdateMode,
    data: Vec<Category>,
}

impl CategoriesUpdate {
    /// Update/add the listed categories, keep the rest (`action: "merge"`).
    pub fn merge<I: IntoIterator<Item = Category>>(categories: I) -> Self {
        Self { mode: UpdateMode::Merge, data: categories.into_iter().collect() }
    }

    /// Replace the entire category list (`action: "replace"`). Destructive.
    pub fn replace_all<I: IntoIterator<Item = Category>>(categories: I) -> Self {
        Self { mode: UpdateMode::ReplaceAll, data: categories.into_iter().collect() }
    }
}

#[derive(Clone, Debug)]
pub struct ReportFieldsUpdate {
    mode: UpdateMode,
    data: Vec<ReportFieldDef>,
}

impl ReportFieldsUpdate {
    pub fn merge<I: IntoIterator<Item = ReportFieldDef>>(fields: I) -> Self {
        Self { mode: UpdateMode::Merge, data: fields.into_iter().collect() }
    }

    pub fn replace_all<I: IntoIterator<Item = ReportFieldDef>>(fields: I) -> Self {
        Self { mode: UpdateMode::ReplaceAll, data: fields.into_iter().collect() }
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
    pub tags: Vec<PolicyTag>,
}

impl TagLevel {
    pub fn new<I: IntoIterator<Item = PolicyTag>>(tags: I) -> Self {
        Self { name: None, required: false, tags: tags.into_iter().collect() }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

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
    dependent: bool,
    set_required: Vec<bool>,
    gl_codes: bool,
    header_row: bool,
    tsv: bool,
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

    pub fn tsv(mut self) -> Self {
        self.tsv = true;
        self
    }
}

#[derive(Clone, Debug)]
enum TagsSource {
    Inline(Vec<TagLevel>),
    Csv { data: Bytes, config: TagCsvConfig },
}

#[derive(Clone, Debug)]
pub struct TagsUpdate {
    mode: UpdateMode,
    source: TagsSource,
}

impl TagsUpdate {
    pub fn merge_inline<I: IntoIterator<Item = TagLevel>>(levels: I) -> Self {
        Self {
            mode: UpdateMode::Merge,
            source: TagsSource::Inline(levels.into_iter().collect()),
        }
    }

    pub fn replace_all_inline<I: IntoIterator<Item = TagLevel>>(levels: I) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            source: TagsSource::Inline(levels.into_iter().collect()),
        }
    }

    /// Tag data uploaded in the separate `file` form field.
    pub fn merge_csv(data: impl Into<Bytes>, config: TagCsvConfig) -> Self {
        Self {
            mode: UpdateMode::Merge,
            source: TagsSource::Csv { data: data.into(), config },
        }
    }

    pub fn replace_all_csv(data: impl Into<Bytes>, config: TagCsvConfig) -> Self {
        Self {
            mode: UpdateMode::ReplaceAll,
            source: TagsSource::Csv { data: data.into(), config },
        }
    }
}

/// Policy Updater (`type: "update"`, `inputSettings.type: "policy"`).
/// Sections are independent; set any subset. Awaiting with no section set
/// is a no-op request the server accepts.
#[must_use = "actions do nothing until awaited"]
pub struct UpdatePolicyAction {
    client: Client,
    policy_ids: Vec<PolicyId>,
    categories: Option<CategoriesUpdate>,
    report_fields: Option<ReportFieldsUpdate>,
    tags: Option<TagsUpdate>,
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

    pub fn categories(mut self, update: CategoriesUpdate) -> Self {
        self.categories = Some(update);
        self
    }

    pub fn report_fields(mut self, update: ReportFieldsUpdate) -> Self {
        self.report_fields = Some(update);
        self
    }

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
            let _ = self;
            todo!()
        })
    }
}
