//! A stable identifier for the *shape* of a failure.
//!
//! Two agents hitting one defect describe it two ways — "decode error on
//! export", "expected value at line 1 column 1" — and neither search finds the
//! other's issue. This turns the description into a lookup key: the same
//! defect answers the same token this month and next, so searching for it is
//! exact rather than a guess.
//!
//! # What goes into it
//!
//! Command path, exit code and error discriminant, and deliberately nothing
//! else. No crate version, no timestamp, no line number and no message text —
//! a fingerprint that moved when a message was reworded or a release was cut
//! would dedup nothing.

use std::fmt;

use expensify::{ApiErrorKind, DecodeError, Error};

use crate::auth::CredentialError;

/// A failure worth reporting, and the token to report it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defect {
    pub id: String,
    pub command: &'static str,
    pub exit: u8,
    pub kind: &'static str,
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}  [{} exit={} {}]",
            self.id, self.command, self.exit, self.kind
        )
    }
}

/// `None` when the failure is not this client's to answer for — see [`kind`].
pub fn identify(command: &'static str, exit: u8, err: &anyhow::Error) -> Option<Defect> {
    let kind = kind(err)?;
    Some(Defect {
        id: id(command, exit, kind),
        command,
        exit,
        kind,
    })
}

/// Which failures carry a fingerprint at all.
///
/// A fingerprint says "this client could not make sense of what happened", so
/// it is withheld wherever something else already explained the failure:
/// Expensify refusing the credentials, rejecting the request or asking for a
/// pause; the CLI settling it before a request went out; the network. Those
/// are facts about an account, a request or a machine, and filing them would
/// bury the defects under them.
///
/// What is left is the young-client failure mode this repository actually
/// wants reported: a response it could not read, an HTTP failure it could not
/// place, a `responseCode` it does not classify, and anything that reached the
/// top with no explanation at all.
fn kind(err: &anyhow::Error) -> Option<&'static str> {
    if err.chain().any(|cause| cause.is::<CredentialError>()) {
        return None;
    }
    match err.chain().find_map(|cause| cause.downcast_ref::<Error>()) {
        Some(Error::Decode(decode)) => Some(match decode {
            DecodeError::Json(_) => "decode.json",
            DecodeError::Utf8(_) => "decode.utf8",
            // `Custom` and anything added later. The variant is the stable
            // part; its message is not, so several custom decode failures on
            // one command share a fingerprint by design.
            _ => "decode.other",
        }),
        Some(Error::Http { .. }) => Some("http"),
        Some(Error::Api(api)) if api.kind == ApiErrorKind::Other => Some("api.other"),
        Some(_) => None,
        // Never reached the wire. A file that would not read, JSON that would
        // not parse, a path that could not be written — the environment's
        // problem. Anything else got here unexplained, which is a defect.
        None => (!err
            .chain()
            .any(|cause| cause.is::<std::io::Error>() || cause.is::<serde_json::Error>()))
        .then_some("unclassified"),
    }
}

/// The definition of the fingerprint. **Changing this line renumbers every
/// filed defect**, which is the one thing this is for.
fn id(command: &str, exit: u8, kind: &str) -> String {
    let hashed = fnv1a(format!("{command}|{exit}|{kind}").as_bytes());
    // Folded to 32 bits: short enough to read back over a call, wide enough
    // that the few dozen shapes this CLI can produce will not collide.
    format!("EXP-{:08X}", ((hashed >> 32) ^ hashed) as u32)
}

/// FNV-1a, hand-rolled for one reason: `DefaultHasher` is explicitly not
/// stable across Rust releases, and stability across releases is the property
/// being bought here.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use expensify::ApiError;

    fn decode_failure() -> anyhow::Error {
        let err: Error = DecodeError::Json(serde_json::from_str::<i32>("nope").unwrap_err()).into();
        anyhow::Error::from(err).context("reading policies")
    }

    /// The pinned values. A change here is a change to every issue already
    /// filed under the old token, so this test failing means the scheme
    /// moved — not that the expectations are stale.
    #[test]
    fn fingerprints_are_pinned_to_exact_values() {
        assert_eq!(id("export.reports", 10, "decode.json"), "EXP-9CAE0FE8");
        assert_eq!(id("get.policies", 10, "decode.json"), "EXP-BAE7E423");
        assert_eq!(id("download", 10, "http"), "EXP-209DBB18");
    }

    #[test]
    fn one_defect_answers_one_id_however_it_is_dressed() {
        let bare = identify("get.policies", 10, &decode_failure()).unwrap();
        let wrapped = identify(
            "get.policies",
            10,
            &decode_failure()
                .context("while doing the thing")
                .context("and another"),
        )
        .unwrap();

        assert_eq!(bare, wrapped);
        assert_eq!(bare.kind, "decode.json");
        assert!(bare.id.starts_with("EXP-"), "{bare}");
    }

    #[test]
    fn different_failures_do_not_collide() {
        let mut seen = std::collections::HashSet::new();
        for command in ["get.policies", "export.reports", "download"] {
            for kind in ["decode.json", "decode.utf8", "decode.other", "http"] {
                assert!(seen.insert(id(command, 10, kind)), "{command}/{kind}");
            }
        }
        assert_ne!(id("download", 10, "http"), id("download", 1, "http"));
    }

    /// The exclusions, each of which has something better to say for itself
    /// than a defect report.
    #[test]
    fn explained_failures_carry_no_fingerprint() {
        let explained = [
            anyhow::Error::from(CredentialError("nothing stored".to_owned())),
            anyhow::Error::from(Error::Api(ApiError {
                kind: ApiErrorKind::InvalidPermissions,
                code: 403,
                message: None,
            })),
            anyhow::Error::from(Error::Api(ApiError {
                kind: ApiErrorKind::Validation,
                code: 410,
                message: None,
            })),
            anyhow::Error::from(Error::RateLimited { retry_after: None }),
            anyhow::Error::from(Error::InvalidRequest("empty selection".to_owned())),
            anyhow::Error::from(std::io::Error::other("disk full")).context("writing july.json"),
        ];
        for err in &explained {
            assert_eq!(identify("get.policies", 4, err), None, "{err:#}");
        }
    }

    #[test]
    fn an_unexplained_failure_is_a_defect() {
        let defect = identify("reimburse", 1, &anyhow::anyhow!("boom")).unwrap();
        assert_eq!(defect.kind, "unclassified");

        let other = identify(
            "get.cards",
            1,
            &anyhow::Error::from(Error::Api(ApiError {
                kind: ApiErrorKind::Other,
                code: 666,
                message: Some("Rule already exists".to_owned()),
            })),
        )
        .unwrap();
        assert_eq!(other.kind, "api.other");
    }

    /// It has to survive being copied out of a terminal by eye.
    #[test]
    fn the_rendered_line_carries_the_token_and_its_inputs() {
        let rendered = identify("get.policies", 10, &decode_failure())
            .unwrap()
            .to_string();
        assert!(rendered.starts_with("EXP-"), "{rendered}");
        assert!(rendered.contains("get.policies"), "{rendered}");
        assert!(rendered.contains("exit=10"), "{rendered}");
        assert!(rendered.contains("decode.json"), "{rendered}");
    }
}
