use std::fmt;
use std::marker::PhantomData;

use time::Date;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::file::{ExportedFile, FileSystem};
use crate::template::ExportTemplate;
use crate::types::{PolicyId, ReportId};
use crate::wire;

/// Which reports an export selects.
///
/// The constructors anchor Expensify's "at least one of reportIDList /
/// startDate / approvedAfter" requirement, so there is no way to *spell* a
/// filterless query. One hole is left, and it is closed at runtime rather
/// than by the type: `report_ids([])` type-checks and anchors nothing, so
/// awaiting the export returns [`Error::InvalidRequest`] instead of letting
/// the server answer 410.
#[derive(Clone, Debug)]
pub struct ReportsQuery {
    pub(crate) report_ids: Vec<ReportId>,
    pub(crate) start_date: Option<Date>,
    pub(crate) end_date: Option<Date>,
    pub(crate) approved_after: Option<Date>,
    pub(crate) policy_ids: Vec<PolicyId>,
    pub(crate) marked_as_exported: Option<String>,
}

impl ReportsQuery {
    /// Specific reports (`filters.reportIDList`).
    pub fn report_ids<I>(ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<ReportId>,
    {
        Self {
            report_ids: ids.into_iter().map(Into::into).collect(),
            start_date: None,
            end_date: None,
            approved_after: None,
            policy_ids: Vec::new(),
            marked_as_exported: None,
        }
    }

    /// Reports created on or after `start` (`filters.startDate`).
    pub fn since(start: Date) -> Self {
        Self {
            report_ids: Vec::new(),
            start_date: Some(start),
            end_date: None,
            approved_after: None,
            policy_ids: Vec::new(),
            marked_as_exported: None,
        }
    }

    /// Reports approved after `date` (`filters.approvedAfter`).
    pub fn approved_after(date: Date) -> Self {
        Self {
            report_ids: Vec::new(),
            start_date: None,
            end_date: None,
            approved_after: Some(date),
            policy_ids: Vec::new(),
            marked_as_exported: None,
        }
    }

    /// `filters.endDate`. Server rule: required when the start anchor is
    /// over a year old; span may not exceed one year (server-validated).
    pub fn until(mut self, end: Date) -> Self {
        self.end_date = Some(end);
        self
    }

    /// Restrict to these policies (`filters.policyIDList`).
    pub fn policy_ids<I>(mut self, ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        self.policy_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Exclude reports already marked exported under `label`
    /// (`filters.markedAsExported`).
    pub fn not_yet_exported_as(mut self, label: impl Into<String>) -> Self {
        self.marked_as_exported = Some(label.into());
        self
    }

    /// `policy_ids` and `not_yet_exported_as` narrow a selection; they do
    /// not make one.
    pub(crate) fn anchored(&self) -> bool {
        !self.report_ids.is_empty() || self.start_date.is_some() || self.approved_after.is_some()
    }
}

/// A report's workflow state (`reportState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportState {
    /// Draft, not yet submitted.
    Open,
    /// Submitted, awaiting approval.
    Submitted,
    /// Approved, not yet reimbursed.
    Approved,
    /// Reimbursed.
    Reimbursed,
    /// Archived.
    Archived,
}

/// `outputSettings.fileExtension`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// Comma-separated values (the server default).
    Csv,
    /// Legacy Excel workbook.
    Xls,
    /// Excel workbook.
    Xlsx,
    /// Plain text.
    Txt,
    /// PDF — note that Expensify emits one file *per report*, which this
    /// crate's single-[`ExportedFile`] model does not represent.
    Pdf,
    /// JSON; pair with a [`Json`](crate::Json) template marker.
    Json,
    /// XML.
    Xml,
}

/// SFTP endpoint, shared by the exporter's `sftpUpload` action and the
/// employee updater's SFTP feed source.
///
/// [`fmt::Debug`] redacts [`SftpConnection::password`], for the same reason
/// [`Credentials`](crate::Credentials) does: this type is reachable from
/// the `Debug` of [`OnFinish`], every export action, and
/// [`EmployeeSource`](crate::EmployeeSource).
#[derive(Clone)]
pub struct SftpConnection {
    /// Hostname or IP.
    pub host: String,
    /// Username.
    pub login: String,
    /// Password. Never printed by [`fmt::Debug`].
    pub password: String,
    /// Port, usually 22.
    pub port: u16,
}

impl fmt::Debug for SftpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SftpConnection")
            .field("host", &self.host)
            .field("login", &self.login)
            .field("password", &"<redacted>")
            .field("port", &self.port)
            .finish()
    }
}

/// An `onFinish` action for the Report Exporter.
#[derive(Clone, Debug)]
pub struct OnFinish {
    pub(crate) kind: OnFinishKind,
}

#[derive(Clone, Debug)]
pub(crate) enum OnFinishKind {
    MarkAsExported {
        label: String,
    },
    Email {
        recipients: String,
        message: Option<String>,
    },
    SftpUpload(SftpConnection),
}

impl OnFinish {
    /// Tag the exported reports with `label` so a later
    /// [`ReportsQuery::not_yet_exported_as`] skips them.
    pub fn mark_as_exported(label: impl Into<String>) -> Self {
        Self {
            kind: OnFinishKind::MarkAsExported {
                label: label.into(),
            },
        }
    }

    /// Comma-separate multiple recipients (wire format).
    pub fn email(recipients: impl Into<String>) -> Self {
        Self {
            kind: OnFinishKind::Email {
                recipients: recipients.into(),
                message: None,
            },
        }
    }

    /// Only meaningful on [`OnFinish::email`].
    pub fn message(mut self, message: impl Into<String>) -> Self {
        if let OnFinishKind::Email { message: m, .. } = &mut self.kind {
            *m = Some(message.into());
        }
        self
    }

    /// Upload the rendered file to an SFTP server. The destination folder
    /// is the SFTP user's home directory; there is no path parameter.
    pub fn sftp_upload(connection: SftpConnection) -> Self {
        Self {
            kind: OnFinishKind::SftpUpload(connection),
        }
    }
}

/// Report Exporter job (`type: "file"`, `inputSettings.type:
/// "combinedReportData"`). Awaiting submits the job with
/// `onReceive.immediateResponse: ["returnRandomFileName"]` and resolves to
/// the generated file handle; the export itself completes asynchronously
/// server-side.
#[must_use = "actions do nothing until awaited"]
pub struct ExportReportsAction<F> {
    pub(crate) client: Client,
    pub(crate) template: String,
    pub(crate) query: ReportsQuery,
    pub(crate) states: Vec<ReportState>,
    pub(crate) limit: Option<u32>,
    pub(crate) employee_email: Option<String>,
    pub(crate) format: Option<ExportFormat>,
    pub(crate) file_basename: Option<String>,
    pub(crate) include_full_page_receipts_pdf: bool,
    pub(crate) on_finish: Vec<OnFinish>,
    pub(crate) test: bool,
    _out: PhantomData<fn() -> F>,
}

impl<F> ExportReportsAction<F> {
    pub(crate) fn new(client: Client, template: &ExportTemplate<F>, query: ReportsQuery) -> Self {
        Self {
            client,
            template: template.source().to_owned(),
            query,
            states: Vec::new(),
            limit: None,
            employee_email: None,
            format: None,
            file_basename: None,
            include_full_page_receipts_pdf: false,
            on_finish: Vec::new(),
            test: false,
            _out: PhantomData,
        }
    }

    /// Restrict by report state; repeatable (`reportState` is
    /// comma-separated on the wire). Default: all states.
    pub fn state(mut self, state: ReportState) -> Self {
        self.states.push(state);
        self
    }

    /// Cap the number of exported reports.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Export a single employee's reports.
    ///
    /// Restricted: Expensify must have granted the credential access to the
    /// employee's domain, otherwise the job fails with
    /// [`ApiErrorKind::InvalidPermissions`](crate::ApiErrorKind::InvalidPermissions).
    /// It also blocks exporting OPEN reports.
    pub fn employee_email(mut self, email: impl Into<String>) -> Self {
        self.employee_email = Some(email.into());
        self
    }

    /// Default: [`ExportFormat::Csv`] for every template marker, including
    /// [`Json`](crate::Json) — the format is not derived from the marker.
    pub fn format(mut self, format: ExportFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Filename stem (`outputSettings.fileBasename`, default `export`).
    /// Expensify appends a random suffix regardless.
    pub fn file_basename(mut self, basename: impl Into<String>) -> Self {
        self.file_basename = Some(basename.into());
        self
    }

    /// PDF exports only: embed full-page receipt images.
    pub fn include_full_page_receipts_pdf(mut self) -> Self {
        self.include_full_page_receipts_pdf = true;
        self
    }

    /// Append an `onFinish` action; repeatable.
    pub fn on_finish(mut self, action: OnFinish) -> Self {
        self.on_finish.push(action);
        self
    }

    /// Sugar for the most common `onFinish` action.
    pub fn mark_as_exported(self, label: impl Into<String>) -> Self {
        self.on_finish(OnFinish::mark_as_exported(label))
    }

    /// Sets `test`: Expensify skips all `onFinish` actions.
    ///
    /// Sent as the string `"true"`, which is how Expensify's parameter table
    /// types it. Not yet confirmed against a live account — if the server
    /// were boolean-typed instead, this would be a silent no-op and
    /// `markAsExported` (irreversible through this API) would fire.
    pub fn test_run(mut self) -> Self {
        self.test = true;
        self
    }
}

impl<F: 'static> IntoFuture for ExportReportsAction<F> {
    type Output = Result<ExportedFile<F>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if !self.query.anchored() {
                return Err(Error::InvalidRequest(
                    "export needs at least one of report IDs, `since` or `approved_after`; \
                     an empty `filters` is a documented 410"
                        .to_owned(),
                ));
            }
            let request = wire::export_reports(&self);
            let response = self.client.send(request).await?;
            let name = wire::filename(response)?;
            // The producer pins the file system; download never asks.
            Ok(ExportedFile::from_response(
                name,
                FileSystem::IntegrationServer,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same rule as `Credentials`: a password must not reach a log line,
    /// including through the `Debug` of anything that holds it.
    #[test]
    fn debug_redacts_the_sftp_password() {
        let connection = SftpConnection {
            host: "sftp.acme.com".into(),
            login: "acme".into(),
            password: "hunter2-super-secret".into(),
            port: 22,
        };
        for rendered in [
            format!("{connection:?}"),
            format!("{:?}", OnFinish::sftp_upload(connection.clone())),
        ] {
            assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
            assert!(rendered.contains("<redacted>"), "{rendered}");
            assert!(rendered.contains("sftp.acme.com"), "{rendered}");
        }
    }
}
