//! Turning failures into a sentence and an exit code.

use expensify::{ApiErrorKind, Error};

use crate::auth::CredentialError;

pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_NO_CREDENTIALS: u8 = 3;
pub const EXIT_PERMISSION_DENIED: u8 = 4;
pub const EXIT_NOT_FOUND: u8 = 5;
pub const EXIT_INVALID_REQUEST: u8 = 6;
pub const EXIT_RATE_LIMITED: u8 = 7;
pub const EXIT_PARTIAL: u8 = 8;
pub const EXIT_NETWORK: u8 = 9;
pub const EXIT_BAD_RESPONSE: u8 = 10;

const CREDENTIALS_URL: &str = "https://www.expensify.com/tools/integrations/";

pub fn exit_code(err: &anyhow::Error) -> u8 {
    if err.chain().any(|cause| cause.is::<CredentialError>()) {
        return EXIT_NO_CREDENTIALS;
    }
    match err.chain().find_map(|cause| cause.downcast_ref::<Error>()) {
        None => EXIT_FAILURE,
        Some(Error::Api(api)) => match api.kind {
            ApiErrorKind::InvalidPermissions => EXIT_PERMISSION_DENIED,
            ApiErrorKind::NotFound => EXIT_NOT_FOUND,
            ApiErrorKind::Validation => EXIT_INVALID_REQUEST,
            _ => EXIT_FAILURE,
        },
        Some(Error::InvalidRequest(_)) => EXIT_INVALID_REQUEST,
        Some(Error::RateLimited { .. }) => EXIT_RATE_LIMITED,
        Some(Error::PartialSuccess(_)) => EXIT_PARTIAL,
        Some(Error::Transport(_)) => EXIT_NETWORK,
        Some(Error::Decode(_)) => EXIT_BAD_RESPONSE,
        Some(Error::Http { .. }) => EXIT_BAD_RESPONSE,
        Some(_) => EXIT_FAILURE,
    }
}

/// One `error:` line, the cause chain, then advice where there is any.
pub fn report(err: &anyhow::Error) {
    eprintln!("error: {err}");
    for cause in err.chain().skip(1) {
        eprintln!("  caused by: {cause}");
    }
    if let Some(advice) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<Error>())
        .and_then(advice)
    {
        eprintln!();
        eprintln!("{advice}");
    }
}

/// What the user can actually do about it. `None` where the message above
/// already says everything.
fn advice(err: &Error) -> Option<String> {
    match err {
        Error::Api(api) => match api.kind {
            ApiErrorKind::InvalidPermissions => Some(format!(
                "Expensify refused the credentials for this operation.\n\
                 Policy reads and writes need policy-admin rights; reconciliation, the \
                 card list\nand the employee updater need domain-admin rights. Check the \
                 pair's account at\n{CREDENTIALS_URL}, then check that account's roles in \
                 Expensify."
            )),
            ApiErrorKind::NotFound => Some(
                "Expensify has no such policy or report, or these credentials cannot see \
                 it.\nList what they can see with `expensify get policies`."
                    .to_owned(),
            ),
            ApiErrorKind::Validation => Some(
                "Expensify rejected the request itself. The message above is its \
                 explanation;\ndates must be YYYY-MM-DD, windows may not exceed a year, \
                 and an end date is\nrequired once the start is over a year old."
                    .to_owned(),
            ),
            ApiErrorKind::Server => Some(format!(
                "Expensify reported a server-side failure. It also uses this code for \
                 operations\nits support team has not enabled for your account — report \
                 creation and acting\non another employee's behalf both need that. \
                 Credentials: {CREDENTIALS_URL}"
            )),
            _ => None,
        },
        Error::RateLimited { retry_after } => Some(match retry_after {
            Some(wait) => format!(
                "Expensify asked for a {} second pause. The built-in limiter paces one \
                 process;\nseveral processes sharing one credential pair need pacing of \
                 their own.",
                wait.as_secs()
            ),
            None => "Expensify allows 5 requests per 10 seconds and 20 per 60. The \
                     built-in limiter\npaces one process; several processes sharing one \
                     credential pair need pacing of\ntheir own."
                .to_owned(),
        }),
        Error::PartialSuccess(outcome) => Some(format!(
            "{} report(s) were updated, {} skipped and {} failed. Re-run with \
             --tolerate-partial\nto see the full breakdown, or narrow the selection to \
             the reports that failed.",
            outcome.updated.len(),
            outcome.skipped.len(),
            outcome.failed.len()
        )),
        Error::Decode(_) => Some(
            "Expensify's response did not match what this client expects. For an export, \
             the\nlikeliest cause is a template whose output does not match --format."
                .to_owned(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expensify::ApiError;

    fn api(kind: ApiErrorKind, code: u16) -> anyhow::Error {
        anyhow::Error::from(Error::Api(ApiError {
            kind,
            code,
            message: Some("nope".to_owned()),
        }))
    }

    #[test]
    fn api_error_kinds_get_distinct_exit_codes() {
        assert_eq!(
            exit_code(&api(ApiErrorKind::InvalidPermissions, 403)),
            EXIT_PERMISSION_DENIED
        );
        assert_eq!(exit_code(&api(ApiErrorKind::NotFound, 404)), EXIT_NOT_FOUND);
        assert_eq!(
            exit_code(&api(ApiErrorKind::Validation, 410)),
            EXIT_INVALID_REQUEST
        );
    }

    #[test]
    fn missing_credentials_have_their_own_code() {
        let err = anyhow::Error::from(CredentialError("nothing stored".to_owned()))
            .context("while listing policies");
        assert_eq!(exit_code(&err), EXIT_NO_CREDENTIALS);
    }

    /// The exit code must survive the `.context()` calls the commands add.
    #[test]
    fn context_does_not_hide_the_api_error() {
        let err = api(ApiErrorKind::InvalidPermissions, 403).context("while reading policy P1");
        assert_eq!(exit_code(&err), EXIT_PERMISSION_DENIED);
    }

    #[test]
    fn permission_advice_points_at_the_credential_page() {
        let err = Error::Api(ApiError {
            kind: ApiErrorKind::InvalidPermissions,
            code: 403,
            message: None,
        });
        let advice = advice(&err).expect("permissions errors carry advice");
        assert!(advice.contains(CREDENTIALS_URL), "{advice}");
        assert!(!advice.contains("Api {"), "no Debug dumps: {advice}");
    }

    #[test]
    fn an_unmapped_error_still_exits_nonzero() {
        assert_eq!(exit_code(&anyhow::anyhow!("boom")), EXIT_FAILURE);
    }
}
