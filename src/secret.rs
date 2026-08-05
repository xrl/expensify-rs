//! Types that make redaction structural.
//!
//! Two shapes of secret occur in this API, and they want different
//! treatment:
//!
//! - [`Secret<T>`] — the whole value is the secret (a partner secret, an SFTP
//!   password). Nothing about it is printable, so nothing is printed.
//! - [`MaskedUrl`] — the value is mostly public and carries a secret
//!   *substring* (`https://user:pass@host/feed.json`). Redacting the whole
//!   thing would delete the half you print a URL for, so only the userinfo
//!   goes.
//!
//! Both redact in `Debug` and `Display` and require a named accessor
//! ([`Secret::expose`], [`MaskedUrl::expose`]) to reach the raw value. That is
//! the whole mechanism: a field typed this way cannot leak through a derived
//! `Debug`, a formatted log line, or `serde` — [`Secret`] deliberately
//! implements no `Serialize`, so putting one on the wire is a call the wire
//! layer has to make on purpose.

use std::fmt;

/// A value whose contents must never reach human-facing output.
///
/// `Debug` and `Display` both render `<redacted>`; the raw value is reachable
/// only through [`Secret::expose`] or [`Secret::into_inner`]. That makes a
/// struct holding one safe to `#[derive(Debug)]`, which is the point: the
/// hand-written `Debug` impl this replaces was correct only for as long as
/// everyone remembered to write it.
///
/// ```
/// use expensify::Secret;
///
/// let secret: Secret<String> = "hunter2".into();
/// assert_eq!(format!("{secret:?}"), "<redacted>");
/// assert_eq!(secret.expose(), "hunter2");
/// ```
///
/// # Not serializable, on purpose
///
/// There is no `Serialize` impl. Redaction is for human-facing output, not for
/// the request body — but the way to keep those apart is to make putting a
/// secret into *any* serializer an explicit act, rather than something a
/// `#[derive(Serialize)]` three types away can do silently. The wire layer
/// calls [`Secret::expose`] at exactly one place per secret.
///
/// # What this does not do
///
/// The value is not locked in memory and is not zeroed on drop; a core dump or
/// a swapped page still contains it. Closing that needs a dependency
/// (`zeroize`) and a guarantee this crate cannot make about `String`
/// reallocation, so it is out of scope.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T = String>(T);

impl<T> Secret<T> {
    /// Wrap a value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the raw value. Named so it reads as a decision at the call site.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Unwrap the raw value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret<String> {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// What every redacting impl in this crate renders.
pub(crate) const REDACTED: &str = "<redacted>";

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// A URL that may embed `user:pass@` userinfo, printed without it.
///
/// `https://hr:pw@acme.com/feed.json` is a natural way to spell a basic-auth
/// feed, which makes the URL a secret carrier — but scheme, host and path are
/// exactly what a printed URL is printed for, so this masks the userinfo and
/// keeps the rest.
///
/// ```
/// use expensify::MaskedUrl;
///
/// let url: MaskedUrl = "https://hr:pw@acme.com/feed.json".into();
/// assert_eq!(url.to_string(), "https://<redacted>@acme.com/feed.json");
/// assert_eq!(url.expose(), "https://hr:pw@acme.com/feed.json");
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct MaskedUrl(String);

impl MaskedUrl {
    /// Wrap a URL string. Unparseable input is accepted as-is: this type
    /// prints, it does not validate.
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// The URL as given, userinfo included.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The URL with any userinfo replaced by `<redacted>`.
    pub fn masked(&self) -> String {
        redact_userinfo(&self.0)
    }
}

impl From<&str> for MaskedUrl {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for MaskedUrl {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&crate::Url> for MaskedUrl {
    fn from(value: &crate::Url) -> Self {
        Self(value.as_str().to_owned())
    }
}

impl fmt::Display for MaskedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.masked())
    }
}

impl fmt::Debug for MaskedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Quoted, so it reads like the `String` it wraps inside a derived
        // `Debug`.
        write!(f, "{:?}", self.masked())
    }
}

/// Replace a URL's `user:pass@` with `<redacted>@`, keeping scheme, host and
/// path.
///
/// Hand-rolled rather than via `Url`: the inputs are caller-supplied strings
/// that may not parse, and `url::Url`'s own `Debug` prints the password.
fn redact_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority = scheme_end + 3;
    let authority_end = url[authority..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |offset| authority + offset);
    match url[authority..authority_end].rfind('@') {
        None => url.to_owned(),
        Some(at) => {
            let mut redacted = String::with_capacity(url.len());
            redacted.push_str(&url[..authority]);
            redacted.push_str(REDACTED);
            redacted.push('@');
            redacted.push_str(&url[authority + at + 1..]);
            redacted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    #[allow(dead_code)]
    struct Holder {
        name: String,
        password: Secret<String>,
    }

    #[test]
    fn a_derived_debug_cannot_print_a_secret() {
        let holder = Holder {
            name: "acme".into(),
            password: "hunter2-super-secret".into(),
        };
        for rendered in [format!("{holder:?}"), format!("{holder:#?}")] {
            assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
            assert!(rendered.contains(REDACTED), "{rendered}");
            assert!(rendered.contains("acme"), "{rendered}");
        }
    }

    #[test]
    fn display_redacts_too() {
        let secret: Secret<String> = "hunter2-super-secret".into();
        assert_eq!(secret.to_string(), REDACTED);
        assert_eq!(secret.expose(), "hunter2-super-secret");
        assert_eq!(secret.into_inner(), "hunter2-super-secret");
    }

    #[test]
    fn masked_urls_keep_the_debuggable_half() {
        let url = MaskedUrl::from("https://hr:pw@acme.com/feed.json?x=1");
        assert_eq!(url.to_string(), "https://<redacted>@acme.com/feed.json?x=1");
        assert_eq!(
            format!("{url:?}"),
            r#""https://<redacted>@acme.com/feed.json?x=1""#
        );

        // An `@` in the path is not userinfo.
        assert_eq!(
            MaskedUrl::from("https://acme.com/feed@latest.json").masked(),
            "https://acme.com/feed@latest.json"
        );
        assert_eq!(MaskedUrl::from("not a url").masked(), "not a url");
    }
}
