//! Request/response observability.
//!
//! Install an [`Observer`] on the client with
//! [`ClientBuilder::observe`](crate::ClientBuilder::observe) and every job —
//! not a chosen few — reports what it sent and what came back:
//!
//! ```no_run
//! # async fn f() -> Result<(), expensify::Error> {
//! use expensify::{Client, Credentials, Recorder};
//!
//! let recorder = Recorder::new();
//! let client = Client::builder(Credentials::new("id", "secret"))
//!     .observe(recorder.clone())
//!     .build();
//!
//! let _ = client.list_policies().await;
//!
//! for exchange in recorder.take() {
//!     println!("{} -> {}", exchange.request().job_type(), exchange.status());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Nothing is observed by default and nothing is rendered when no observer is
//! installed, so the cost of the feature when unused is one `Option` check per
//! request.
//!
//! # Credentials
//!
//! An observed request body can never contain the partner secret, the SFTP
//! password or the employee-feed password. That is not a filter over the
//! finished body: secrets enter the job description as opaque placeholders and
//! are substituted in only when the outgoing body is rendered, so the
//! observable rendering is built from a structure that has never held one.
//!
//! # Personal data
//!
//! **Response bodies are reproduced verbatim and routinely contain personal
//! data** — employee names, email addresses, manager relationships, masked
//! card numbers, merchant and amount detail. Anything derived from an
//! [`Exchange`] is as sensitive as the account it came from, so treat a
//! captured exchange (or a verbose log pasted into a ticket) accordingly.

use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use reqwest::StatusCode;

use crate::secret::MaskedUrl;

/// Receives every request and response the client makes.
///
/// Implement it on your own type, or pass any `Fn(&Exchange)` closure — the
/// blanket impl covers [`Observer::on_exchange`] and leaves
/// [`Observer::on_request`] at its no-op default.
///
/// Both methods run inline on the task driving the request, so an
/// implementation that blocks stalls that request. Capture cheaply
/// ([`Recorder`] pushes to a `Vec`) and do the expensive part afterwards.
///
/// See the [module docs](self#personal-data): response bodies carry personal
/// data.
pub trait Observer: Send + Sync + 'static {
    /// Called immediately before the request is sent — so a request that
    /// never comes back is still visible.
    fn on_request(&self, request: &ObservedRequest) {
        let _ = request;
    }

    /// Called once the response body has been read, whatever the status.
    ///
    /// A request that fails at the transport layer (connection refused, TLS
    /// failure, timeout) produces no exchange; it surfaced through
    /// [`on_request`](Observer::on_request) and is returned to the caller as
    /// [`Error::Transport`](crate::Error::Transport).
    fn on_exchange(&self, exchange: &Exchange);
}

impl<F> Observer for F
where
    F: Fn(&Exchange) + Send + Sync + 'static,
{
    fn on_exchange(&self, exchange: &Exchange) {
        self(exchange);
    }
}

/// A request as it went out, with every secret already removed.
///
/// The fields are the form fields of the POST body before URL encoding:
/// `requestJobDescription` always, plus `template`, `file` or `data` for the
/// jobs that use them. Encoding is where bugs are not, and the decoded form is
/// what you would paste into `curl --data-urlencode`.
#[derive(Clone)]
pub struct ObservedRequest {
    url: MaskedUrl,
    job_type: String,
    fields: Vec<(&'static str, String)>,
}

impl ObservedRequest {
    pub(crate) fn new(
        url: MaskedUrl,
        job_type: String,
        fields: Vec<(&'static str, String)>,
    ) -> Self {
        Self {
            url,
            job_type,
            fields,
        }
    }

    /// The endpoint posted to, with any userinfo masked.
    pub fn url(&self) -> &MaskedUrl {
        &self.url
    }

    /// The Expensify job type (`file`, `get`, `update`, `download`, ...) —
    /// the discriminator every job shares one endpoint behind.
    pub fn job_type(&self) -> &str {
        &self.job_type
    }

    /// The `requestJobDescription` JSON, credentials redacted.
    pub fn job_description(&self) -> &str {
        self.field("requestJobDescription").unwrap_or_default()
    }

    /// Every form field, in the order sent.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.fields.iter().map(|(k, v)| (*k, v.as_str()))
    }

    /// One form field by name.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

impl fmt::Display for ObservedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "POST {} [job {}]", self.url, self.job_type)?;
        for (name, value) in &self.fields {
            write!(f, "\n  {name}={value}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ObservedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedRequest")
            .field("url", &self.url)
            .field("job_type", &self.job_type)
            .field("fields", &self.fields)
            .finish()
    }
}

/// One completed round trip: the request as sent and the response as
/// received.
///
/// The response body is the raw bytes off the wire, before this crate decides
/// whether they are an envelope, a file, or neither — which is the point, when
/// what you are debugging is that decision. See the
/// [module docs](self#personal-data) on personal data.
#[derive(Clone)]
pub struct Exchange {
    request: ObservedRequest,
    status: StatusCode,
    content_type: Option<String>,
    body: Bytes,
    duration: Duration,
}

impl Exchange {
    pub(crate) fn new(
        request: ObservedRequest,
        status: StatusCode,
        content_type: Option<String>,
        body: Bytes,
        duration: Duration,
    ) -> Self {
        Self {
            request,
            status,
            content_type,
            body,
            duration,
        }
    }

    /// The request that produced this response.
    pub fn request(&self) -> &ObservedRequest {
        &self.request
    }

    /// HTTP status. Expensify answers 200 for most failures — the body's
    /// `responseCode` is the one that counts.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// `Content-Type` header, verbatim (`application/json`,
    /// `text/plain;charset=UTF-8`, ...). `None` when the response carried
    /// none or it was not valid UTF-8.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The response body as received.
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// The response body as text, with invalid UTF-8 replaced.
    pub fn body_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    /// Wall time from just before the request was sent to the last byte of
    /// the response body — excluding any wait on the built-in rate limiter.
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.request)?;
        write!(
            f,
            "\n  -> {} {} in {:?}, {} bytes",
            self.status.as_u16(),
            self.content_type.as_deref().unwrap_or("(no content-type)"),
            self.duration,
            self.body.len(),
        )?;
        write!(f, "\n  {}", self.body_text())
    }
}

impl fmt::Debug for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exchange")
            .field("request", &self.request)
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("duration", &self.duration)
            .field("body", &self.body_text())
            .finish()
    }
}

/// An [`Observer`] that keeps exchanges in memory.
///
/// Cheap to clone (shares one buffer), so the copy you hand to
/// [`ClientBuilder::observe`](crate::ClientBuilder::observe) and the copy you
/// read from are the same recorder. This is the intended base for recording
/// live responses as test fixtures: run the real call once, [`Recorder::take`]
/// the exchanges, and write each [`Exchange::body`] to a file keyed by
/// [`ObservedRequest::job_type`].
pub struct Recorder {
    exchanges: Arc<Mutex<Vec<Exchange>>>,
}

impl Recorder {
    /// A new, empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of everything recorded so far.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.lock().clone()
    }

    /// Everything recorded so far, leaving the recorder empty.
    pub fn take(&self) -> Vec<Exchange> {
        std::mem::take(&mut *self.lock())
    }

    /// How many exchanges are held.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// An observer that panicked mid-record must not poison every later
    /// request; the buffer is a `Vec` and cannot be left inconsistent.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Exchange>> {
        self.exchanges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            exchanges: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Clone for Recorder {
    fn clone(&self) -> Self {
        Self {
            exchanges: Arc::clone(&self.exchanges),
        }
    }
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("exchanges", &self.len())
            .finish()
    }
}

impl Observer for Recorder {
    fn on_exchange(&self, exchange: &Exchange) {
        self.lock().push(exchange.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(body: &'static str) -> Exchange {
        Exchange::new(
            ObservedRequest::new(
                MaskedUrl::from("https://proxy:pw@gw.acme.com/expensify"),
                "file".to_owned(),
                vec![("requestJobDescription", r#"{"type":"file"}"#.to_owned())],
            ),
            StatusCode::OK,
            Some("text/plain".to_owned()),
            Bytes::from_static(body.as_bytes()),
            Duration::from_millis(12),
        )
    }

    #[test]
    fn a_recorder_shares_one_buffer() {
        let recorder = Recorder::new();
        let installed = recorder.clone();
        assert!(recorder.is_empty());

        installed.on_exchange(&exchange("a"));
        installed.on_exchange(&exchange("b"));

        assert_eq!(recorder.len(), 2);
        assert_eq!(recorder.take().len(), 2);
        assert!(recorder.is_empty());
    }

    #[test]
    fn closures_are_observers() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let observer = move |exchange: &Exchange| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(exchange.body_text().into_owned());
        };
        observer.on_exchange(&exchange("hello"));
        assert_eq!(seen.lock().unwrap().as_slice(), ["hello".to_owned()]);
    }

    #[test]
    fn rendering_an_exchange_masks_the_endpoint_userinfo() {
        let rendered = format!("{}", exchange("ok"));
        assert!(!rendered.contains("proxy:pw@"), "{rendered}");
        assert!(rendered.contains("<redacted>@gw.acme.com"), "{rendered}");
        assert!(rendered.contains("text/plain"), "{rendered}");
        assert!(rendered.contains("ok"), "{rendered}");
    }
}
