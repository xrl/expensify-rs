use std::marker::PhantomData;

use time::Date;

use crate::client::Client;
use crate::error::Error;
use crate::file::{ExportedFile, FileSystem};
use crate::template::ExportTemplate;
use crate::types::{PolicyId, ReportId};
use crate::BoxFuture;

/// Which reports an export selects. Constructors anchor the "at least one
/// of reportIDList / startDate / approvedAfter" requirement: an empty
/// query is unrepresentable.
#[derive(Clone, Debug)]
pub struct ReportsQuery {
    report_ids: Vec<ReportId>,
    start_date: Option<Date>,
    end_date: Option<Date>,
    approved_after: Option<Date>,
    policy_ids: Vec<PolicyId>,
    marked_as_exported: Option<String>,
}

impl ReportsQuery {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportState {
    Open,
    Submitted,
    Approved,
    Reimbursed,
    Archived,
}

/// `outputSettings.fileExtension`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Xls,
    Xlsx,
    Txt,
    Pdf,
    Json,
    Xml,
}

#[derive(Clone, Debug)]
pub struct SftpConnection {
    pub host: String,
    pub login: String,
    pub password: String,
    pub port: u16,
}

/// An `onFinish` action for the Report Exporter.
#[derive(Clone, Debug)]
pub struct OnFinish {
    kind: OnFinishKind,
}

#[derive(Clone, Debug)]
enum OnFinishKind {
    MarkAsExported { label: String },
    Email { recipients: String, message: Option<String> },
    SftpUpload(SftpConnection),
}

impl OnFinish {
    pub fn mark_as_exported(label: impl Into<String>) -> Self {
        Self { kind: OnFinishKind::MarkAsExported { label: label.into() } }
    }

    /// Comma-separate multiple recipients (wire format).
    pub fn email(recipients: impl Into<String>) -> Self {
        Self {
            kind: OnFinishKind::Email { recipients: recipients.into(), message: None },
        }
    }

    /// Only meaningful on [`OnFinish::email`].
    pub fn message(mut self, message: impl Into<String>) -> Self {
        if let OnFinishKind::Email { message: m, .. } = &mut self.kind {
            *m = Some(message.into());
        }
        self
    }

    pub fn sftp_upload(connection: SftpConnection) -> Self {
        Self { kind: OnFinishKind::SftpUpload(connection) }
    }
}

/// Report Exporter job (`type: "file"`, `inputSettings.type:
/// "combinedReportData"`). Awaiting submits the job with
/// `onReceive.immediateResponse: ["returnRandomFileName"]` and resolves to
/// the generated file handle; the export itself completes asynchronously
/// server-side.
#[must_use = "actions do nothing until awaited"]
pub struct ExportReportsAction<F> {
    client: Client,
    template: String,
    query: ReportsQuery,
    states: Vec<ReportState>,
    limit: Option<u32>,
    employee_email: Option<String>,
    format: Option<ExportFormat>,
    file_basename: Option<String>,
    include_full_page_receipts_pdf: bool,
    on_finish: Vec<OnFinish>,
    test: bool,
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

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Export a single employee's reports. Restricted: requires Expensify
    /// to have granted the credential access to the employee's domain, and
    /// blocks exporting OPEN reports.
    pub fn employee_email(mut self, email: impl Into<String>) -> Self {
        self.employee_email = Some(email.into());
        self
    }

    /// Default: [`ExportFormat::Csv`] for untyped templates,
    /// [`ExportFormat::Json`] when the template marker is [`crate::Json`].
    pub fn format(mut self, format: ExportFormat) -> Self {
        self.format = Some(format);
        self
    }

    pub fn file_basename(mut self, basename: impl Into<String>) -> Self {
        self.file_basename = Some(basename.into());
        self
    }

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

    /// Sets `test: "true"`: Expensify skips all `onFinish` actions.
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
            let _ = FileSystem::IntegrationServer; // producer pins the file system
            let _ = self;
            todo!()
        })
    }
}
