//! Credential resolution and keychain storage.
//!
//! Three sources, in order: command-line flags, environment, OS keychain.
//! A source that supplies one half of the pair must supply the other, so a
//! stale keychain entry can never silently pair with a fresh environment
//! variable.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const ENV_ID: &str = "EXPENSIFY_PARTNER_USER_ID";
pub const ENV_SECRET: &str = "EXPENSIFY_PARTNER_USER_SECRET";

const KEYCHAIN_SERVICE: &str = "expensify-cli";
const KEYCHAIN_ACCOUNT: &str = "partner-credentials";

/// Where `expensify auth status` says a credential came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Flags,
    Environment,
    Keychain,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Flags => "command-line flags",
            Self::Environment => "environment",
            Self::Keychain => "OS keychain",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub partner_user_id: String,
    pub partner_user_secret: String,
    pub source: Source,
}

/// Stored as one JSON blob so the pair is written and cleared atomically.
#[derive(Debug, Deserialize, Serialize)]
struct StoredCredentials {
    partner_user_id: String,
    partner_user_secret: String,
}

/// Read side of the environment, injectable so the precedence rule is
/// testable without mutating process state.
pub trait Env {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// The keychain, injectable for the same reason — and because CI has none.
pub trait SecretStore {
    fn load(&self) -> Result<Option<(String, String)>>;
    fn save(&self, partner_user_id: &str, partner_user_secret: &str) -> Result<()>;
    /// `false` when there was nothing stored.
    fn clear(&self) -> Result<bool>;
}

pub struct Keychain;

impl Keychain {
    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|err| {
            anyhow::anyhow!("no usable OS keychain ({err}); set {ENV_ID} and {ENV_SECRET} instead")
        })
    }
}

impl SecretStore for Keychain {
    fn load(&self) -> Result<Option<(String, String)>> {
        match self.entry()?.get_password() {
            Ok(blob) => {
                let stored: StoredCredentials = serde_json::from_str(&blob).map_err(|err| {
                    anyhow::anyhow!(
                        "the stored credential is not readable ({err}); \
                         run `expensify auth login` again"
                    )
                })?;
                Ok(Some((stored.partner_user_id, stored.partner_user_secret)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(anyhow::anyhow!("could not read the OS keychain: {err}")),
        }
    }

    fn save(&self, partner_user_id: &str, partner_user_secret: &str) -> Result<()> {
        let blob = serde_json::to_string(&StoredCredentials {
            partner_user_id: partner_user_id.to_owned(),
            partner_user_secret: partner_user_secret.to_owned(),
        })?;
        self.entry()?
            .set_password(&blob)
            .map_err(|err| anyhow::anyhow!("could not write to the OS keychain: {err}"))
    }

    fn clear(&self) -> Result<bool> {
        match self.entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(anyhow::anyhow!("could not clear the OS keychain: {err}")),
        }
    }
}

/// Flags, then environment, then keychain.
pub fn resolve(
    flag_id: Option<&str>,
    flag_secret: Option<&str>,
    env: &dyn Env,
    store: &dyn SecretStore,
) -> Result<Resolved> {
    if flag_id.is_some() || flag_secret.is_some() {
        let (id, secret) = require_pair(
            flag_id,
            flag_secret,
            "--partner-user-id",
            "--partner-user-secret",
        )?;
        return Ok(Resolved {
            partner_user_id: id,
            partner_user_secret: secret,
            source: Source::Flags,
        });
    }

    let env_id = env.get(ENV_ID);
    let env_secret = env.get(ENV_SECRET);
    if env_id.is_some() || env_secret.is_some() {
        let (id, secret) =
            require_pair(env_id.as_deref(), env_secret.as_deref(), ENV_ID, ENV_SECRET)?;
        return Ok(Resolved {
            partner_user_id: id,
            partner_user_secret: secret,
            source: Source::Environment,
        });
    }

    match store.load()? {
        Some((id, secret)) => Ok(Resolved {
            partner_user_id: id,
            partner_user_secret: secret,
            source: Source::Keychain,
        }),
        None => bail!(CredentialError(format!(
            "no credentials: run `expensify auth login`, or set {ENV_ID} and {ENV_SECRET}"
        ))),
    }
}

/// Its own type so `main` can map every "cannot authenticate" case to one
/// exit code without matching on message strings.
#[derive(Debug)]
pub struct CredentialError(pub String);

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CredentialError {}

fn require_pair(
    id: Option<&str>,
    secret: Option<&str>,
    id_name: &str,
    secret_name: &str,
) -> Result<(String, String)> {
    match (id, secret) {
        (Some(id), Some(secret)) => Ok((id.to_owned(), secret.to_owned())),
        (Some(_), None) => bail!(CredentialError(format!(
            "{id_name} is set but {secret_name} is not; set both"
        ))),
        (None, Some(_)) => bail!(CredentialError(format!(
            "{secret_name} is set but {id_name} is not; set both"
        ))),
        (None, None) => unreachable!("callers check that one half is present"),
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub struct FakeEnv(pub HashMap<String, String>);

    impl FakeEnv {
        pub fn empty() -> Self {
            Self(HashMap::new())
        }

        pub fn with(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            )
        }
    }

    impl Env for FakeEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    #[derive(Default)]
    pub struct FakeStore(pub RefCell<Option<(String, String)>>);

    impl FakeStore {
        pub fn holding(id: &str, secret: &str) -> Self {
            Self(RefCell::new(Some((id.to_owned(), secret.to_owned()))))
        }
    }

    impl SecretStore for FakeStore {
        fn load(&self) -> Result<Option<(String, String)>> {
            Ok(self.0.borrow().clone())
        }

        fn save(&self, id: &str, secret: &str) -> Result<()> {
            *self.0.borrow_mut() = Some((id.to_owned(), secret.to_owned()));
            Ok(())
        }

        fn clear(&self) -> Result<bool> {
            Ok(self.0.borrow_mut().take().is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{FakeEnv, FakeStore};
    use super::*;

    #[test]
    fn environment_beats_the_keychain() {
        let env = FakeEnv::with(&[(ENV_ID, "env-id"), (ENV_SECRET, "env-secret")]);
        let store = FakeStore::holding("keychain-id", "keychain-secret");

        let resolved = resolve(None, None, &env, &store).unwrap();

        assert_eq!(resolved.partner_user_id, "env-id");
        assert_eq!(resolved.partner_user_secret, "env-secret");
        assert_eq!(resolved.source, Source::Environment);
    }

    #[test]
    fn flags_beat_the_environment() {
        let env = FakeEnv::with(&[(ENV_ID, "env-id"), (ENV_SECRET, "env-secret")]);
        let store = FakeStore::holding("keychain-id", "keychain-secret");

        let resolved = resolve(Some("flag-id"), Some("flag-secret"), &env, &store).unwrap();

        assert_eq!(resolved.partner_user_id, "flag-id");
        assert_eq!(resolved.source, Source::Flags);
    }

    #[test]
    fn the_keychain_is_the_last_resort() {
        let resolved = resolve(
            None,
            None,
            &FakeEnv::empty(),
            &FakeStore::holding("keychain-id", "keychain-secret"),
        )
        .unwrap();

        assert_eq!(resolved.partner_user_id, "keychain-id");
        assert_eq!(resolved.source, Source::Keychain);
    }

    /// The mixing guard: a half-set source must not fall through to the
    /// next one and pair credentials from two different accounts.
    #[test]
    fn a_half_set_source_is_an_error_not_a_fallthrough() {
        let store = FakeStore::holding("keychain-id", "keychain-secret");

        let err = resolve(None, None, &FakeEnv::with(&[(ENV_ID, "env-id")]), &store).unwrap_err();
        assert!(err.to_string().contains(ENV_SECRET), "{err}");

        let err = resolve(Some("flag-id"), None, &FakeEnv::empty(), &store).unwrap_err();
        assert!(err.to_string().contains("--partner-user-secret"), "{err}");
    }

    #[test]
    fn every_credential_failure_is_the_same_typed_error() {
        for err in [
            resolve(None, None, &FakeEnv::empty(), &FakeStore::default()).unwrap_err(),
            resolve(Some("id"), None, &FakeEnv::empty(), &FakeStore::default()).unwrap_err(),
        ] {
            assert!(err.downcast_ref::<CredentialError>().is_some(), "{err}");
        }
    }

    #[test]
    fn stored_credentials_round_trip() {
        let store = FakeStore::default();
        store.save("id", "secret").unwrap();
        assert_eq!(
            store.load().unwrap(),
            Some(("id".to_owned(), "secret".to_owned()))
        );
        assert!(store.clear().unwrap());
        assert!(!store.clear().unwrap());
    }
}
