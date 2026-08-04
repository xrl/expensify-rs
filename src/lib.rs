//! Typed client for the [Expensify Integration Server API][api].
//!
//! One HTTP endpoint serves every Expensify "job"; this crate gives each job
//! a method on [`Client`] that returns a `#[must_use]` action struct. Required
//! arguments go in the method call, optional ones are fluent setters, and the
//! action executes when you `.await` it:
//!
//! ```no_run
//! # async fn f() -> Result<(), expensify::Error> {
//! use expensify::{Client, Credentials, ReimburseTargets};
//!
//! let client = Client::new(Credentials::new("partner-id", "partner-secret"));
//!
//! let updated = client
//!     .mark_reports_reimbursed(ReimburseTargets::report_ids(["R006AseGxMka"]))
//!     .payment_source("ACME-AP")
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Typed exports
//!
//! Export output shape is defined by the caller's FreeMarker template, and the
//! Downloader's file system must match the job that produced the filename.
//! Both facts are carried in one phantom chain — template to
//! [`ExportedFile`] to download result — so a mismatched decode or a
//! reconciliation filename fetched from the wrong file system is a compile
//! error rather than a runtime surprise. See [`FromExport`] and [`Json`].
//!
//! # Requested-field typestate
//!
//! [`Client::get_policies`] returns data only for the sections you asked for.
//! Each `with_*` call flips a type-level flag, so unrequested sections hold an
//! inert [`NotFetched`] placeholder and reading one does not compile. There is
//! no `Option` to unwrap.
//!
//! # Errors
//!
//! Expensify signals failure through a `responseCode` inside an HTTP 200 body
//! at least as often as through the status line. The wire layer always reads
//! the body envelope first, so [`Error::Api`] is produced consistently
//! regardless of which layer carried the code.
//!
//! # Rate limiting
//!
//! Expensify allows 5 requests per 10 seconds and 20 per 60 seconds. A
//! matching two-window limiter is on by default; disable it with
//! [`ClientBuilder::no_rate_limiting`] when an external governor owns the
//! budget. The limiter is process-local, so 429s remain a surfaced error.
//!
//! [api]: https://integrations.expensify.com/Integration-Server/doc/
#![deny(missing_docs)]

mod cards;
mod client;
mod employees;
mod error;
mod expense_rules;
mod expenses;
mod export;
mod file;
mod limit;
mod policy;
mod reconciliation;
mod reports;
mod template;
mod types;
mod wire;

pub use cards::*;
pub use client::*;
pub use employees::*;
pub use error::*;
pub use expense_rules::*;
pub use expenses::*;
pub use export::*;
pub use file::*;
pub use policy::*;
pub use reconciliation::*;
pub use reports::*;
pub use template::*;
pub use types::*;

pub(crate) type BoxFuture<T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;
