//! Credential resolution and keychain storage.
//!
//! Three sources, in order: command-line flags, environment, OS keychain.
//! A source that supplies one half of the pair must supply the other, so a
//! stale keychain entry can never silently pair with a fresh environment
//! variable.

use std::io::IsTerminal as _;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Result, bail};
use expensify::Secret;
use serde::{Deserialize, Serialize};

pub const ENV_ID: &str = "EXPENSIFY_PARTNER_USER_ID";
pub const ENV_SECRET: &str = "EXPENSIFY_PARTNER_USER_SECRET";
pub const ENV_KEYCHAIN_TIMEOUT: &str = "EXPENSIFY_KEYCHAIN_TIMEOUT_SECS";

const KEYCHAIN_SERVICE: &str = "expensify-cli";
const KEYCHAIN_ACCOUNT: &str = "partner-credentials";

/// Long enough for someone to notice a permission dialog, find their login
/// password and answer it.
const ATTENDED_WAIT: Duration = Duration::from_secs(120);

/// Nobody is going to answer a dialog here, so the only question left is how
/// long a *working* keychain may reasonably take.
const UNATTENDED_WAIT: Duration = Duration::from_secs(10);

/// How long the keychain may take before the wait itself is worth mentioning.
const SILENT_WAIT: Duration = Duration::from_secs(1);

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
    pub partner_user_secret: Secret<String>,
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

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|err| {
        anyhow::anyhow!("no usable OS keychain ({err}); set {ENV_ID} and {ENV_SECRET} instead")
    })
}

impl SecretStore for Keychain {
    fn load(&self) -> Result<Option<(String, String)>> {
        bounded("read", || match entry()?.get_password() {
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
        })
    }

    fn save(&self, partner_user_id: &str, partner_user_secret: &str) -> Result<()> {
        let blob = serde_json::to_string(&StoredCredentials {
            partner_user_id: partner_user_id.to_owned(),
            partner_user_secret: partner_user_secret.to_owned(),
        })?;
        bounded("write to", move || {
            entry()?
                .set_password(&blob)
                .map_err(|err| anyhow::anyhow!("could not write to the OS keychain: {err}"))
        })
    }

    fn clear(&self) -> Result<bool> {
        bounded("clear", || match entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(anyhow::anyhow!("could not clear the OS keychain: {err}")),
        })
    }
}

/// A keychain call that gives up instead of blocking forever.
///
/// macOS grants keychain access per *executable*, so a binary it has not seen
/// before raises a permission prompt on its first call — and every
/// `cargo build` produces one. With nothing attached to answer the prompt the
/// underlying call never returns, and the process sits silent indefinitely.
fn bounded<T: Send + 'static>(
    verb: &str,
    call: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let limit = wait_limit(attended(), &ProcessEnv);
    match wait_for(limit, SILENT_WAIT, call) {
        Some(result) => result,
        None => bail!(CredentialError(gave_up(verb, limit))),
    }
}

/// Runs `call` on a thread this function is willing to abandon.
///
/// Abandoning it is the whole mechanism: the blocked call is inside the
/// platform keychain library and there is nothing to cancel. The thread holds
/// nothing the rest of the process needs and dies with it.
fn wait_for<T: Send + 'static>(
    limit: Option<Duration>,
    silent: Duration,
    call: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(call());
    });

    let silent = limit.map_or(silent, |limit| silent.min(limit));
    match rx.recv_timeout(silent) {
        Ok(value) => return Some(value),
        Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        Err(mpsc::RecvTimeoutError::Timeout) => {}
    }

    // Suppressed by --quiet, which sets our own ceiling to ERROR.
    tracing::warn!("waiting for the OS keychain — approve the permission prompt if one appeared");
    match limit {
        None => rx.recv().ok(),
        Some(limit) => rx.recv_timeout(limit - silent).ok(),
    }
}

/// A human who could answer a permission prompt shows up, to a process, as a
/// terminal on both ends: somewhere to show the wait, and something that
/// could have typed the command. A background agent, a CI runner and
/// `ssh host expensify ...` have neither, and are the cases that hang.
fn attended() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// `None` waits indefinitely, which is what the pre-timeout CLI did.
fn wait_limit(attended: bool, env: &dyn Env) -> Option<Duration> {
    let default = if attended {
        ATTENDED_WAIT
    } else {
        UNATTENDED_WAIT
    };
    match env.get(ENV_KEYCHAIN_TIMEOUT) {
        None => Some(default),
        Some(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(seconds) => Some(Duration::from_secs(seconds)),
            Err(_) => {
                tracing::warn!("ignoring {ENV_KEYCHAIN_TIMEOUT}={raw}: expected whole seconds");
                Some(default)
            }
        },
    }
}

fn gave_up(verb: &str, limit: Option<Duration>) -> String {
    let waited = match limit {
        Some(limit) => format!(" within {}s", limit.as_secs()),
        None => String::new(),
    };
    format!(
        "could not {verb} the OS keychain{waited}.\n\
         Keychain access is granted per executable, so a binary the OS has not seen \
         before — every `cargo build` produces one — raises a permission prompt, and \
         nothing here can answer it. Any of:\n  \
         - re-run this from a terminal and approve the prompt\n  \
         - set {ENV_ID} and {ENV_SECRET}\n  \
         - `expensify auth login` to store the pair from this binary\n  \
         - {ENV_KEYCHAIN_TIMEOUT}=0 to wait indefinitely"
    )
}

/// The identity a failure should be attributed to.
///
/// Written once, when a command resolves credentials; read once, by the error
/// reporter after that command has given up. Global because the two are a
/// whole command apart and every path in between would otherwise have to carry
/// it.
static ACCOUNT: OnceLock<Account> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct Account {
    pub partner_user_id: String,
    pub source: Source,
}

pub fn remember(resolved: &Resolved) {
    let _ = ACCOUNT.set(Account {
        partner_user_id: resolved.partner_user_id.clone(),
        source: resolved.source,
    });
}

/// The account this process authenticated as, if it got that far.
pub fn account() -> Option<&'static Account> {
    ACCOUNT.get()
}

/// Flags, then environment, then keychain.
pub fn resolve(
    flag_id: Option<&str>,
    flag_secret: Option<&Secret<String>>,
    env: &dyn Env,
    store: &dyn SecretStore,
) -> Result<Resolved> {
    if flag_id.is_some() || flag_secret.is_some() {
        let (id, secret) = require_pair(
            flag_id,
            flag_secret.map(|secret| secret.expose().as_str()),
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
            partner_user_secret: secret.into(),
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
) -> Result<(String, Secret<String>)> {
    match (id, secret) {
        (Some(id), Some(secret)) => Ok((id.to_owned(), secret.into())),
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
        assert_eq!(resolved.partner_user_secret.expose(), "env-secret");
        assert_eq!(resolved.source, Source::Environment);
    }

    #[test]
    fn flags_beat_the_environment() {
        let env = FakeEnv::with(&[(ENV_ID, "env-id"), (ENV_SECRET, "env-secret")]);
        let store = FakeStore::holding("keychain-id", "keychain-secret");

        let resolved = resolve(Some("flag-id"), Some(&"flag-secret".into()), &env, &store).unwrap();

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

    /// The hang this exists for: a keychain call that never answers.
    #[test]
    fn a_keychain_that_never_answers_is_given_up_on() {
        let waited = std::time::Instant::now();
        let outcome = wait_for(
            Some(Duration::from_millis(60)),
            Duration::from_millis(10),
            || {
                std::thread::sleep(Duration::from_secs(30));
                "answered at last"
            },
        );

        assert_eq!(outcome, None);
        assert!(waited.elapsed() < Duration::from_secs(5), "it blocked");
    }

    #[test]
    fn a_keychain_that_answers_in_time_is_waited_for() {
        // Longer than the silent window, so the "still waiting" branch is the
        // one under test — the fast path would not exercise it.
        let outcome = wait_for(
            Some(Duration::from_secs(30)),
            Duration::from_millis(10),
            || {
                std::thread::sleep(Duration::from_millis(50));
                "answered"
            },
        );
        assert_eq!(outcome, Some("answered"));
    }

    /// A terminal means someone can click Allow; the generous limit is what
    /// keeps that working setup working.
    #[test]
    fn an_attended_terminal_waits_far_longer_than_an_unattended_one() {
        let env = FakeEnv::empty();
        assert_eq!(wait_limit(true, &env), Some(ATTENDED_WAIT));
        assert_eq!(wait_limit(false, &env), Some(UNATTENDED_WAIT));
        assert!(ATTENDED_WAIT > UNATTENDED_WAIT);
    }

    #[test]
    fn the_limit_can_be_overridden_and_switched_off() {
        let seconds = FakeEnv::with(&[(ENV_KEYCHAIN_TIMEOUT, "3")]);
        assert_eq!(wait_limit(false, &seconds), Some(Duration::from_secs(3)));

        let forever = FakeEnv::with(&[(ENV_KEYCHAIN_TIMEOUT, "0")]);
        assert_eq!(wait_limit(false, &forever), None);

        // A diagnostic knob set wrong must not itself become the failure.
        let nonsense = FakeEnv::with(&[(ENV_KEYCHAIN_TIMEOUT, "soon")]);
        assert_eq!(wait_limit(false, &nonsense), Some(UNATTENDED_WAIT));
    }

    /// The message is the whole point of the fix: it has to name every way
    /// out, because the one thing the caller knows is that nothing happened.
    #[test]
    fn giving_up_names_the_ways_out() {
        let message = gave_up("read", Some(UNATTENDED_WAIT));
        for expected in [
            "10s",
            ENV_ID,
            ENV_SECRET,
            ENV_KEYCHAIN_TIMEOUT,
            "auth login",
            "per executable",
        ] {
            assert!(message.contains(expected), "{message}");
        }
        assert!(
            !gave_up("read", None).contains("within"),
            "an unbounded wait has no elapsed limit to report"
        );
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
