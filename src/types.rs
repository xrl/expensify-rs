use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

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

string_id!(PolicyId);
string_id!(ReportId);
string_id!(TransactionId);
string_id!(TaxRateId);

/// Expense-rule identifier (`ruleID`). Integer on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(pub i64);

/// Three-letter currency code, e.g. "USD". Not validated client-side.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Currency(String);

impl Currency {
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

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

/// An amount in minor units (cents) paired with its currency.
/// Expensify amounts are always integer cents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub cents: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(cents: i64, currency: impl Into<Currency>) -> Self {
        Self { cents, currency: currency.into() }
    }
}
