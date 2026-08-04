use std::fmt;
use std::marker::PhantomData;

use bytes::Bytes;
use serde::de::DeserializeOwned;

use crate::error::DecodeError;

/// How a downloaded export body is turned into a Rust value.
///
/// Implemented on *marker* types, not on the output itself: the marker is
/// carried as a phantom parameter from template to exported file to
/// download, so `Self::Output` can differ from `Self` (see [`Json`]).
///
/// Open for user implementation (e.g. a CSV marker in the caller's crate):
///
/// ```
/// use bytes::Bytes;
/// use expensify::{DecodeError, FromExport};
///
/// struct SemicolonCsv;
///
/// impl FromExport for SemicolonCsv {
///     type Output = Vec<Vec<String>>;
///
///     fn from_export(bytes: Bytes) -> Result<Self::Output, DecodeError> {
///         let text = String::from_utf8(bytes.to_vec())?;
///         Ok(text
///             .lines()
///             .map(|line| line.split(';').map(str::to_owned).collect())
///             .collect())
///     }
/// }
/// ```
pub trait FromExport {
    /// What a download of this template's output resolves to.
    type Output: Send + 'static;

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
/// The default [`ExportFormat`](crate::ExportFormat) is still
/// [`Csv`](crate::ExportFormat::Csv), so a `Json<_>` template must also call
/// `.format(ExportFormat::Json)` — the mismatch shows up as a decode error
/// on download rather than as silently corrupt data.
pub struct Json<T>(PhantomData<fn() -> T>);

impl<T: DeserializeOwned + Send + 'static> FromExport for Json<T> {
    type Output = T;

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
