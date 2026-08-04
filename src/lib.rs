//! Typed client for the Expensify Integration Server API.
//!
//! Design skeleton: signatures are authoritative, bodies are stubs.
//! See docs/DESIGN.md.
#![allow(dead_code, unused_variables)]

mod cards;
mod client;
mod employees;
mod error;
mod expense_rules;
mod expenses;
mod export;
mod file;
mod policy;
mod reconciliation;
mod reports;
mod template;
mod types;

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
