use std::marker::PhantomData;

use time::Date;

use crate::client::Client;
use crate::error::Error;
use crate::export::ExportFormat;
use crate::file::{ExportedFile, FileSystem};
use crate::template::ReconciliationTemplate;
use crate::BoxFuture;

/// `inputSettings.type` of the reconciliation job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationScope {
    /// Card transactions not yet on any report.
    Unreported,
    /// All card transactions in the window.
    All,
}

/// Reconciliation job (`type: "reconciliation"`). Domain-admin credentials
/// required (server-enforced). Synchronous server-side (`async: false` is
/// the only supported mode and is not exposed); the resolved
/// [`ExportedFile`] is immediately downloadable.
#[must_use = "actions do nothing until awaited"]
pub struct ReconcileAction<F> {
    client: Client,
    domain: String,
    template: String,
    start: Date,
    end: Date,
    scope: ReconciliationScope,
    feed: Option<String>,
    format: Option<ExportFormat>,
    email_on_finish: Option<String>,
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

    /// Only Csv, Txt, Json, Xml are valid here (server-validated).
    /// Default: Csv.
    pub fn format(mut self, format: ExportFormat) -> Self {
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
            let _ = FileSystem::Reconciliation; // producer pins the file system
            let _ = self;
            todo!()
        })
    }
}
