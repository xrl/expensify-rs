use crate::reports::ReimburseOutcome;

/// All fallible library operations return this.
///
/// Expensify signals errors through a `responseCode` embedded in an
/// HTTP-200 JSON body as often as through the HTTP status line; the wire
/// layer normalizes both into [`Error::Api`] (or [`Error::RateLimited`]).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Connection / TLS / timeout failures below the API layer.
    #[error("transport error")]
    Transport(#[from] reqwest::Error),

    /// HTTP 429 or in-body responseCode 429.
    #[error("rate limited by Expensify")]
    RateLimited {
        /// From the `Retry-After` header, which Expensify often omits.
        retry_after: Option<std::time::Duration>,
    },

    /// Expensify rejected the job (`responseCode` != 200/207, from either
    /// the HTTP status or the JSON body).
    #[error("expensify responseCode {}: {}", .0.code, .0.message.as_deref().unwrap_or("<no message>"))]
    Api(ApiError),

    /// The request was rejected before it was sent, because Expensify
    /// documents it as a 410. Empty collections are the whole population:
    /// an export, reimbursement, or policy read whose only anchor is an
    /// empty iterator serializes to an empty `filters`/`policyIDList`.
    #[error("{0}")]
    InvalidRequest(String),

    /// Non-success HTTP response whose body was not a recognizable
    /// Expensify JSON envelope.
    #[error("HTTP {status}")]
    Http {
        /// Status line of the failed response.
        status: reqwest::StatusCode,
        /// Body as received, lossily decoded as UTF-8.
        body: String,
    },

    /// Failed to decode a response body or a downloaded export
    /// (via [`crate::FromExport`]).
    #[error("decode error")]
    Decode(#[from] DecodeError),

    /// responseCode 207: some reports were updated, others skipped or
    /// failed. Only produced by the strict (default) reimbursement path;
    /// `tolerate_partial()` turns this into an `Ok` outcome instead.
    #[error("partial success: {} updated, {} skipped, {} failed",
            .0.updated.len(), .0.skipped.len(), .0.failed.len())]
    PartialSuccess(Box<ReimburseOutcome>),
}

/// A rejection reported by Expensify itself.
#[derive(Clone, Debug)]
pub struct ApiError {
    /// Coarse classification of [`ApiError::code`].
    pub kind: ApiErrorKind,
    /// Raw `responseCode` (or HTTP status when no body code was present).
    pub code: u16,
    /// Expensify's `responseMessage`, when present.
    pub message: Option<String>,
}

/// Documented Expensify response-code families.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiErrorKind {
    /// 403 — e.g. credentials are not a policy/domain admin.
    InvalidPermissions,
    /// 404 — policy not found.
    NotFound,
    /// 410 — request validation failure.
    Validation,
    /// 500 — Expensify-side error (also used for "not authorized to
    /// authenticate as user", i.e. a capability that support has not
    /// enabled).
    Server,
    /// Any other code Expensify returns.
    Other,
}

/// Failure to turn bytes into a value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// Malformed JSON, or JSON that did not match the expected shape.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A download declared as text was not valid UTF-8.
    #[error("invalid utf-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    /// For user-defined [`crate::FromExport`] implementations.
    #[error("{0}")]
    Custom(String),
}

impl DecodeError {
    /// Build a [`DecodeError::Custom`]; the intended failure path for a
    /// caller-side [`crate::FromExport`] impl.
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}
