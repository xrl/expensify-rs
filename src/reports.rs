use std::marker::PhantomData;

use time::Date;

use crate::client::Client;
use crate::error::Error;
use crate::types::{Money, PolicyId, ReportId};
use crate::BoxFuture;

/// An expense line for the Report Creator. Deliberately narrower than
/// [`crate::Expense`]: the report-creation job only accepts these four
/// fields, so category/tag/etc. cannot be attached here and silently
/// dropped.
#[derive(Clone, Debug)]
pub struct ExpenseLine {
    merchant: String,
    date: Date,
    amount: Money,
}

impl ExpenseLine {
    pub fn new(merchant: impl Into<String>, date: Date, amount: Money) -> Self {
        Self { merchant: merchant.into(), date, amount }
    }
}

/// Report Creator (`type: "create"`, `inputSettings.type: "report"`).
/// Requires Expensify support to have enabled report creation for the
/// domain, and domain+policy admin credentials; a persistent
/// "Not authorized to authenticate as user" [`Error::Api`] means it is
/// not enabled.
#[must_use = "actions do nothing until awaited"]
pub struct CreateReportAction {
    client: Client,
    policy_id: PolicyId,
    employee_email: String,
    title: String,
    fields: serde_json::Map<String, serde_json::Value>,
    fields_error: Option<serde_json::Error>,
    expenses: Vec<ExpenseLine>,
}

impl CreateReportAction {
    pub(crate) fn new(
        client: Client,
        policy_id: PolicyId,
        employee_email: String,
        title: String,
        expenses: Vec<ExpenseLine>,
    ) -> Self {
        Self {
            client,
            policy_id,
            employee_email,
            title,
            fields: serde_json::Map::new(),
            fields_error: None,
            expenses,
        }
    }

    /// Set one report field. Keys are normalized to Expensify's rule
    /// (non-alphanumerics become underscores) before sending.
    pub fn report_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), serde_json::Value::String(value.into()));
        self
    }

    /// Set report fields from any `Serialize` type that serializes to a
    /// JSON object (map or struct). Serialization happens eagerly; a
    /// failure surfaces from the eventual `.await`.
    pub fn report_fields<T: serde::Serialize>(self, fields: &T) -> Self {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct CreatedReport {
    pub report_id: ReportId,
    pub name: String,
}

impl IntoFuture for CreateReportAction {
    type Output = Result<CreatedReport, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}

/// Which reports a reimbursement targets. Anchored constructors:
/// Expensify requires `reportIDList` or `startDate`.
#[derive(Clone, Debug)]
pub struct ReimburseTargets {
    report_ids: Vec<ReportId>,
    start_date: Option<Date>,
    end_date: Option<Date>,
}

impl ReimburseTargets {
    pub fn report_ids<I>(ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<ReportId>,
    {
        Self {
            report_ids: ids.into_iter().map(Into::into).collect(),
            start_date: None,
            end_date: None,
        }
    }

    pub fn since(start: Date) -> Self {
        Self { report_ids: Vec::new(), start_date: Some(start), end_date: None }
    }

    pub fn until(mut self, end: Date) -> Self {
        self.end_date = Some(end);
        self
    }
}

/// Strict mode marker (default): a 207 partial success is an error.
pub struct Strict;

/// Tolerant mode marker: a 207 partial success is an `Ok` outcome.
pub struct Tolerant;

/// Report Status Updater (`type: "update"`, `inputSettings.type:
/// "reportStatus"`). The only supported transition is Approved →
/// Reimbursed, so there is no status parameter.
///
/// By default a 207 (some reports skipped/failed) is
/// [`Error::PartialSuccess`]; [`ReimburseAction::tolerate_partial`]
/// switches the output type to the full [`ReimburseOutcome`] instead.
#[must_use = "actions do nothing until awaited"]
pub struct ReimburseAction<Mode = Strict> {
    client: Client,
    targets: ReimburseTargets,
    payment_source: Option<String>,
    _mode: PhantomData<fn() -> Mode>,
}

impl ReimburseAction<Strict> {
    pub(crate) fn new(client: Client, targets: ReimburseTargets) -> Self {
        Self { client, targets, payment_source: None, _mode: PhantomData }
    }

    /// Accept partial success: skipped and failed reports become data in
    /// the [`ReimburseOutcome`] rather than an error.
    pub fn tolerate_partial(self) -> ReimburseAction<Tolerant> {
        ReimburseAction {
            client: self.client,
            targets: self.targets,
            payment_source: self.payment_source,
            _mode: PhantomData,
        }
    }
}

impl<Mode> ReimburseAction<Mode> {
    /// Free-text payment label (`paymentSource`, 1-100 chars,
    /// server-validated).
    pub fn payment_source(mut self, source: impl Into<String>) -> Self {
        self.payment_source = Some(source.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct SkippedReport {
    pub report_id: ReportId,
    pub reason: String,
}

/// Full outcome of a tolerant reimbursement (also embedded in
/// [`Error::PartialSuccess`] on the strict path).
#[derive(Clone, Debug)]
pub struct ReimburseOutcome {
    pub updated: Vec<ReportId>,
    /// Reports in a non-Approved status.
    pub skipped: Vec<SkippedReport>,
    /// Reports that failed for other reasons.
    pub failed: Vec<SkippedReport>,
}

impl IntoFuture for ReimburseAction<Strict> {
    /// The updated report IDs. A 207 becomes [`Error::PartialSuccess`].
    type Output = Result<Vec<ReportId>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}

impl IntoFuture for ReimburseAction<Tolerant> {
    /// Both 200 and 207 resolve to the outcome.
    type Output = Result<ReimburseOutcome, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}
