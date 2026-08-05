use std::marker::PhantomData;

use time::Date;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::{DecodeError, Error};
use crate::types::{Money, PolicyId, ReportId};
use crate::wire;

/// An expense line for the Report Creator. Deliberately narrower than
/// [`crate::Expense`]: the report-creation job only accepts these four
/// fields, so category/tag/etc. cannot be attached here and silently
/// dropped.
#[derive(Clone, Debug)]
pub struct ExpenseLine {
    pub(crate) merchant: String,
    pub(crate) date: Date,
    pub(crate) amount: Money,
}

impl ExpenseLine {
    /// The only constructor; there are no setters by design.
    pub fn new(merchant: impl Into<String>, date: Date, amount: Money) -> Self {
        Self {
            merchant: merchant.into(),
            date,
            amount,
        }
    }
}

/// Report Creator (`type: "create"`, `inputSettings.type: "report"`).
///
/// Expensify's docs say support must enable report creation for the domain.
/// That requirement is **unconfirmed**: the job worked immediately on a trial
/// account with policy-admin rights and no unlock (observed 2026-08-04). One
/// account is not proof it never applies — a persistent "Not authorized to
/// authenticate as user" [`Error::Api`] is still worth asking support about,
/// since no amount of client-side correctness will fix it.
#[must_use = "actions do nothing until awaited"]
pub struct CreateReportAction {
    pub(crate) client: Client,
    pub(crate) policy_id: PolicyId,
    pub(crate) employee_email: String,
    pub(crate) title: String,
    pub(crate) fields: serde_json::Map<String, serde_json::Value>,
    pub(crate) fields_error: Option<serde_json::Error>,
    pub(crate) expenses: Vec<ExpenseLine>,
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
        self.fields
            .insert(name.into(), serde_json::Value::String(value.into()));
        self
    }

    /// Set report fields from any `Serialize` type that serializes to a
    /// JSON object (map or struct). Serialization happens eagerly; a
    /// failure — including a value that is not an object — surfaces from
    /// the eventual `.await`.
    pub fn report_fields<T: serde::Serialize>(mut self, fields: &T) -> Self {
        match serde_json::to_value(fields) {
            Ok(serde_json::Value::Object(map)) => self.fields.extend(map),
            Ok(_) => {
                self.fields_error.get_or_insert_with(|| {
                    serde::de::Error::custom("report_fields must serialize to a JSON object")
                });
            }
            Err(err) => {
                self.fields_error.get_or_insert(err);
            }
        }
        self
    }
}

/// Result of a successful Report Creator run.
#[derive(Clone, Debug)]
pub struct CreatedReport {
    /// Identifier of the new report.
    pub report_id: ReportId,
    /// Name as stored by Expensify (`reportName`).
    pub name: String,
}

impl IntoFuture for CreateReportAction {
    type Output = Result<CreatedReport, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(mut self) -> Self::IntoFuture {
        Box::pin(async move {
            if let Some(err) = self.fields_error.take() {
                return Err(DecodeError::Json(err).into());
            }
            let request = wire::create_report(&self);
            let response = self.client.send(request).await?;
            wire::created_report(response)
        })
    }
}

/// Which reports a reimbursement targets. Anchored constructors:
/// Expensify requires `reportIDList` or `startDate`.
///
/// `report_ids([])` anchors nothing despite type-checking; awaiting it
/// returns [`Error::InvalidRequest`] rather than sending an empty
/// `filters`.
#[derive(Clone, Debug)]
pub struct ReimburseTargets {
    pub(crate) report_ids: Vec<ReportId>,
    pub(crate) start_date: Option<Date>,
    pub(crate) end_date: Option<Date>,
}

impl ReimburseTargets {
    /// Specific reports.
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

    /// Reports whose latest of submitted/created falls on or after `start`.
    pub fn since(start: Date) -> Self {
        Self {
            report_ids: Vec::new(),
            start_date: Some(start),
            end_date: None,
        }
    }

    /// Close the window (inclusive).
    pub fn until(mut self, end: Date) -> Self {
        self.end_date = Some(end);
        self
    }

    fn anchored(&self) -> bool {
        !self.report_ids.is_empty() || self.start_date.is_some()
    }
}

/// Strict mode marker (default): any partial success is an error.
pub struct Strict;

/// Tolerant mode marker: a partial success is an `Ok` outcome.
pub struct Tolerant;

/// Report Status Updater (`type: "update"`, `inputSettings.type:
/// "reportStatus"`). The only supported transition is Approved →
/// Reimbursed, so there is no status parameter.
///
/// By default any skipped or failed report is [`Error::PartialSuccess`];
/// [`ReimburseAction::tolerate_partial`] switches the output type to the full
/// [`ReimburseOutcome`] instead.
///
/// **Strictness is keyed on the lists, not on the response code**, because
/// the response code does not carry the information. Both partial runs
/// observed live (2026-08-04) came back `responseCode: 200` — one with every
/// report skipped and `reportIDs: []`, one mixed, with a single reimbursed ID
/// beside two skips. Treating 207 as the signal of partial success, which the
/// docs imply and this crate did, made the first `Ok(vec![])` and the second
/// an `Ok` of one ID indistinguishable from a run that fully succeeded, with
/// every reason discarded. 207 is still honoured; it has just never been
/// seen.
#[must_use = "actions do nothing until awaited"]
pub struct ReimburseAction<Mode = Strict> {
    client: Client,
    targets: ReimburseTargets,
    payment_source: Option<String>,
    _mode: PhantomData<fn() -> Mode>,
}

impl ReimburseAction<Strict> {
    pub(crate) fn new(client: Client, targets: ReimburseTargets) -> Self {
        Self {
            client,
            targets,
            payment_source: None,
            _mode: PhantomData,
        }
    }

    /// Accept partial success: skipped and failed reports become data in
    /// the [`ReimburseOutcome`] rather than an error. Read both lists —
    /// Expensify reports a run in which nothing moved as a plain 200.
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

    async fn run(self) -> Result<(bool, ReimburseOutcome), Error> {
        if !self.targets.anchored() {
            return Err(Error::InvalidRequest(
                "reimbursement needs report IDs or `since`; \
                 an empty `filters` is a documented 410"
                    .to_owned(),
            ));
        }
        let request = wire::reimburse(&self.targets, self.payment_source.as_deref());
        let response = self.client.send(request).await?;
        let partial = wire::is_partial(&response);
        Ok((partial, wire::reimburse_outcome(response)?))
    }
}

/// A report the reimbursement did not update, and why.
#[derive(Clone, Debug)]
pub struct SkippedReport {
    /// The report in question.
    pub report_id: ReportId,
    /// Expensify's explanation, e.g. `Report is in status 'Open'`.
    pub reason: String,
}

/// Full outcome of a tolerant reimbursement (also embedded in
/// [`Error::PartialSuccess`] on the strict path).
#[derive(Clone, Debug)]
pub struct ReimburseOutcome {
    /// Reports moved to Reimbursed.
    pub updated: Vec<ReportId>,
    /// Reports in a non-Approved status.
    pub skipped: Vec<SkippedReport>,
    /// Reports that failed for other reasons.
    pub failed: Vec<SkippedReport>,
}

impl IntoFuture for ReimburseAction<Strict> {
    /// The updated report IDs. Any skipped or failed report — or a 207 —
    /// becomes [`Error::PartialSuccess`], so an `Ok` means every targeted
    /// report moved.
    type Output = Result<Vec<ReportId>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (partial, outcome) = self.run().await?;
            if partial || !outcome.skipped.is_empty() || !outcome.failed.is_empty() {
                return Err(Error::PartialSuccess(Box::new(outcome)));
            }
            Ok(outcome.updated)
        })
    }
}

impl IntoFuture for ReimburseAction<Tolerant> {
    /// The full outcome, whatever the response code.
    type Output = Result<ReimburseOutcome, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.run().await.map(|(_, outcome)| outcome) })
    }
}
