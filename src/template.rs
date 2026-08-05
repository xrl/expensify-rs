use std::fmt;
use std::marker::PhantomData;

use bytes::Bytes;
use serde::de::DeserializeOwned;

use crate::error::DecodeError;
use crate::export::ExportFormat;
use crate::reconciliation::ReconciliationFormat;

/// How a downloaded export body is turned into a Rust value.
///
/// Implemented on *marker* types, not on the output itself: the marker is
/// carried as a phantom parameter from template to exported file to
/// download, so `Self::Output` can differ from `Self` (see [`Json`]).
///
/// The two format consts are what the marker says its bytes are, and they
/// supply the export job's default `fileExtension`. Both have a default of
/// `Csv` — the server's own — so an impl that omits them behaves exactly as
/// before; a marker for another format states it:
///
/// ```
/// use bytes::Bytes;
/// use expensify::{DecodeError, ExportFormat, FromExport};
///
/// struct SemicolonCsv;
///
/// impl FromExport for SemicolonCsv {
///     type Output = Vec<Vec<String>>;
///
///     // Omitted: EXPORT_FORMAT and RECONCILIATION_FORMAT both default to Csv.
///
///     fn from_export(bytes: Bytes) -> Result<Self::Output, DecodeError> {
///         let text = String::from_utf8(bytes.to_vec())?;
///         Ok(text
///             .lines()
///             .map(|line| line.split(';').map(str::to_owned).collect())
///             .collect())
///     }
/// }
///
/// struct Xml;
///
/// impl FromExport for Xml {
///     type Output = String;
///     const EXPORT_FORMAT: ExportFormat = ExportFormat::Xml;
///
///     fn from_export(bytes: Bytes) -> Result<String, DecodeError> {
///         Ok(String::from_utf8(bytes.to_vec())?)
///     }
/// }
/// ```
///
/// The two consts are separate because the two jobs accept different
/// vocabularies: reconciliation has no `xls`/`xlsx`, so a marker for a
/// spreadsheet can state a format for the exporter and leave reconciliation
/// at the server default, which is the only truthful thing it can say.
pub trait FromExport {
    /// What a download of this template's output resolves to.
    type Output: Send + 'static;

    /// Default `outputSettings.fileExtension` for
    /// [`Client::export_reports`](crate::Client::export_reports).
    /// An explicit
    /// [`ExportReportsAction::format`](crate::ExportReportsAction::format)
    /// overrides it.
    const EXPORT_FORMAT: ExportFormat = ExportFormat::Csv;

    /// Default `outputSettings.fileExtension` for
    /// [`DomainClient::reconcile`](crate::DomainClient::reconcile).
    /// An explicit
    /// [`ReconcileAction::format`](crate::ReconcileAction::format)
    /// overrides it.
    const RECONCILIATION_FORMAT: ReconciliationFormat = ReconciliationFormat::Csv;

    /// Decode one downloaded export body.
    fn from_export(bytes: Bytes) -> Result<Self::Output, DecodeError>;
}

/// Raw bytes: the escape hatch. `Output = Bytes`, never fails.
impl FromExport for Bytes {
    type Output = Bytes;

    fn from_export(bytes: Bytes) -> Result<Bytes, DecodeError> {
        Ok(bytes)
    }
}

/// UTF-8 text.
impl FromExport for String {
    type Output = String;

    fn from_export(bytes: Bytes) -> Result<String, DecodeError> {
        String::from_utf8(bytes.to_vec()).map_err(Into::into)
    }
}

/// Marker for templates whose output is JSON deserializable into `T`.
/// Never instantiated; exists only at the type level.
///
/// Exporting with this marker defaults `fileExtension` to `json`, so no
/// `.format(ExportFormat::Json)` call is needed. An explicit `.format` still
/// wins; asking for another format is then a stated contradiction, and the
/// download reports it as one.
pub struct Json<T>(PhantomData<fn() -> T>);

impl<T: DeserializeOwned + Send + 'static> FromExport for Json<T> {
    type Output = T;

    const EXPORT_FORMAT: ExportFormat = ExportFormat::Json;
    const RECONCILIATION_FORMAT: ReconciliationFormat = ReconciliationFormat::Json;

    fn from_export(bytes: Bytes) -> Result<T, DecodeError> {
        serde_json::from_slice(&bytes).map_err(Into::into)
    }
}

macro_rules! template_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub struct $name<F = Bytes> {
            source: String,
            _out: PhantomData<fn() -> F>,
        }

        impl $name<Bytes> {
            /// Untyped template: downloads yield raw [`Bytes`].
            pub fn new(source: impl Into<String>) -> Self {
                Self { source: source.into(), _out: PhantomData }
            }
        }

        impl<F: FromExport> $name<F> {
            /// Typed template: declare the output marker (e.g.
            /// `Json<Vec<Row>>`) that downloads of its exports decode into.
            pub fn typed(source: impl Into<String>) -> Self {
                Self { source: source.into(), _out: PhantomData }
            }
        }

        impl<F> $name<F> {
            /// The FreeMarker source as given.
            pub fn source(&self) -> &str {
                &self.source
            }
        }

        // Manual impls: derives would demand `F: Clone/Debug`, which the
        // phantom does not need.
        impl<F> Clone for $name<F> {
            fn clone(&self) -> Self {
                Self { source: self.source.clone(), _out: PhantomData }
            }
        }

        impl<F> fmt::Debug for $name<F> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).field("source", &self.source).finish()
            }
        }
    };
}

template_type! {
    /// FreeMarker template for the Report Exporter data model
    /// (`reports` iteration). Produces files on the `integrationServer`
    /// file system.
    ExportTemplate
}

template_type! {
    /// FreeMarker template for the Reconciliation data model
    /// (`cards` → `reports` → `transactionList` iteration). Produces files
    /// on the `reconciliation` file system. Deliberately a distinct type
    /// from [`ExportTemplate`]: the two template languages evaluate
    /// against disjoint data models.
    ReconciliationTemplate
}
