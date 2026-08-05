//! Command dispatch and the pieces every command shares.

mod auth;
mod completion;
mod create;
mod download;
mod export;
mod get;
mod reimburse;
mod skill;
mod update;

use anyhow::{Context, Result};
use expensify::{Client, Credentials, Url};

use crate::auth::{Keychain, ProcessEnv, resolve};
use crate::cli::{Cli, Command, GlobalArgs};

pub async fn run(cli: Cli) -> Result<()> {
    let Cli { global, command } = cli;
    match command {
        Command::Auth { command } => auth::run(command, &global),
        Command::Get { command } => get::run(command, &global).await,
        Command::Export { command } => export::run(command, &global).await,
        Command::Download(args) => download::run(args, &global).await,
        Command::Create { command } => create::run(command, &global).await,
        Command::Update { command } => update::run(command, &global).await,
        Command::Reimburse(args) => reimburse::run(args, &global).await,
        Command::Completion(args) => completion::run(args),
        Command::Skill { command } => skill::run(command, &global),
    }
}

/// Resolve credentials and build the API client.
pub fn client(global: &GlobalArgs) -> Result<Client> {
    let resolved = resolve(
        global.partner_user_id.as_deref(),
        global.partner_user_secret.as_ref(),
        &ProcessEnv,
        &Keychain,
    )?;
    tracing::debug!(
        source = resolved.source.describe(),
        partner_user_id = resolved.partner_user_id,
        "resolved credentials"
    );

    let mut builder = Client::builder(Credentials::new(
        resolved.partner_user_id,
        resolved.partner_user_secret,
    ));
    if let Some(endpoint) = &global.endpoint {
        let url = Url::parse(endpoint).with_context(|| format!("`{endpoint}` is not a URL"))?;
        builder = builder.base_url(url);
    }
    if global.no_rate_limit {
        builder = builder.no_rate_limiting();
    }
    if global.verbose > 0 {
        if global.verbose > 1 {
            note(
                global,
                "note: -vv prints response bodies verbatim, which routinely carry \
                 personal data (employee names, email addresses, card numbers)",
            );
        }
        builder = builder.observe(crate::observe::Tracing);
    }
    Ok(builder.build())
}

/// A note for a human, suppressed by `--quiet`. Never on stdout: stdout is
/// the result.
pub fn note(global: &GlobalArgs, message: impl std::fmt::Display) {
    if !global.quiet {
        eprintln!("{message}");
    }
}
