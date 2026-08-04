use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wrap an identifier string. No validation: Expensify does not
            /// publish a format.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Borrow the underlying string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<&$name> for $name {
            fn from(s: &$name) -> Self {
                s.clone()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id! {
    /// Policy identifier (`policyID`).
    ///
    /// Distinct from the other id newtypes on purpose: every API surface
    /// takes `impl Into<PolicyId>`, so a string literal works but a
    /// [`ReportId`] does not.
    PolicyId
}
string_id! {
    /// Report identifier (`reportID`), e.g. `R006AseGxMka`.
    ReportId
}
string_id! {
    /// Expense/transaction identifier (`transactionID`).
    TransactionId
}
string_id! {
    /// Tax rate identifier (`rateID`), obtained from
    /// [`Client::get_policies`](crate::Client::get_policies) with
    /// `with_tax()`.
    TaxRateId
}

/// Expense-rule identifier (`ruleID`). Integer on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(pub i64);

/// Three-letter currency code, e.g. "USD". Not validated client-side.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Currency(String);

impl Currency {
    /// Wrap a currency code.
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

    /// Borrow the underlying code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Currency {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for Currency {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&Currency> for Currency {
    fn from(c: &Currency) -> Self {
        c.clone()
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An amount in minor units (cents) paired with its currency.
/// Expensify amounts are always integer cents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// Minor units — `12900` is $129.00, not $12,900.
    pub cents: i64,
    /// ISO-4217-ish code sent alongside the amount.
    pub currency: Currency,
}

impl Money {
    /// Pair an integer-cent amount with its currency.
    pub fn new(cents: i64, currency: impl Into<Currency>) -> Self {
        Self {
            cents,
            currency: currency.into(),
        }
    }
}
