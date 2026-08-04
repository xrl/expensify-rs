use std::marker::PhantomData;

use time::Date;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::file::{ExportedFile, FileSystem};
use crate::template::ReconciliationTemplate;
use crate::wire;

/// `inputSettings.type` of the reconciliation job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationScope {
    /// Card transactions not yet on any report. They appear in the template
    /// data model under a synthetic report with id `0`.
    Unreported,
    /// All card transactions in the window.
    All,
}

/// `outputSettings.fileExtension` for the reconciliation job.
///
/// Narrower than [`ExportFormat`](crate::ExportFormat), which the exporter
/// uses: reconciliation accepts only these four, so the other spellings are
/// unrepresentable rather than server-rejected. Same split as
/// [`ReportFieldDefType`](crate::ReportFieldDefType) vs
/// [`ReportFieldType`](crate::ReportFieldType).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReconciliationFormat {
    /// Comma-separated values (the server default).
    Csv,
    /// Plain text.
    Txt,
    /// JSON; pair with a [`Json`](crate::Json) template marker.
    Json,
    /// XML.
    Xml,
}

/// Reconciliation job (`type: "reconciliation"`). Domain-admin credentials
/// required (server-enforced; a non-admin credential gets
/// [`ApiErrorKind::InvalidPermissions`](crate::ApiErrorKind::InvalidPermissions)).
/// Synchronous server-side (`async: false` is the only supported mode and is
/// not exposed); the resolved [`ExportedFile`] is immediately downloadable.
#[must_use = "actions do nothing until awaited"]
pub struct ReconcileAction<F> {
    pub(crate) client: Client,
    pub(crate) domain: String,
    pub(crate) template: String,
    pub(crate) start: Date,
    pub(crate) end: Date,
    pub(crate) scope: ReconciliationScope,
    pub(crate) feed: Option<String>,
    pub(crate) format: Option<ReconciliationFormat>,
    pub(crate) email_on_finish: Option<String>,
    _out: PhantomData<fn() -> F>,
}

impl<F> ReconcileAction<F> {
    pub(crate) fn new(
        client: Client,
        domain: String,
        template: &ReconciliationTemplate<F>,
        start: Date,
        end: Date,
        scope: ReconciliationScope,
    ) -> Self {
        Self {
            client,
            domain,
            template: template.source().to_owned(),
            start,
            end,
            scope,
            feed: None,
            format: None,
            email_on_finish: None,
            _out: PhantomData,
        }
    }

    /// Restrict to one card feed. Default: all feeds
    /// (`"export_all_feeds"`).
    pub fn feed(mut self, feed: impl Into<String>) -> Self {
        self.feed = Some(feed.into());
        self
    }

    /// Default: [`ReconciliationFormat::Csv`].
    pub fn format(mut self, format: ReconciliationFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Comma-separate multiple recipients.
    pub fn email_on_finish(mut self, recipients: impl Into<String>) -> Self {
        self.email_on_finish = Some(recipients.into());
        self
    }
}

impl<F: 'static> IntoFuture for ReconcileAction<F> {
    type Output = Result<ExportedFile<F>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::reconcile(&self);
            let response = self.client.send(request).await?;
            let name = wire::filename(response)?;
            // The producer pins the file system; download never asks.
            Ok(ExportedFile::from_response(
                name,
                FileSystem::Reconciliation,
            ))
        })
    }
}
