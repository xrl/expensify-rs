//! `expensify auth` — credentials in, out, and where they came from.

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::auth::{ENV_ID, ENV_SECRET, Keychain, ProcessEnv, SecretStore, Source, resolve};
use crate::cli::{AuthCommand, GlobalArgs, LoginArgs};
use crate::output::View;

const CREDENTIALS_URL: &str = "https://www.expensify.com/tools/integrations/";

pub fn run(command: AuthCommand, global: &GlobalArgs) -> Result<()> {
    match command {
        AuthCommand::Login(args) => login(args, global),
        AuthCommand::Status => status(global),
        AuthCommand::Logout => logout(global),
    }
}

fn login(args: LoginArgs, global: &GlobalArgs) -> Result<()> {
    eprintln!("Partner credentials are generated at {CREDENTIALS_URL}");
    eprintln!("Expensify shows the secret exactly once.");

    let partner_user_id = match args.partner_user_id {
        Some(id) => id,
        None => prompt("Partner user ID: ")?,
    };
    if partner_user_id.trim().is_empty() {
        bail!("the partner user ID is empty");
    }

    let secret = rpassword::prompt_password("Partner user secret (not echoed): ")
        .context("reading the secret")?;
    if secret.trim().is_empty() {
        bail!("the partner user secret is empty");
    }

    Keychain.save(partner_user_id.trim(), secret.trim())?;

    View::acknowledgement(
        "credentials",
        format!("stored {} in the OS keychain", partner_user_id.trim()),
    )
    .print(global.output)
}

fn status(global: &GlobalArgs) -> Result<()> {
    let resolved = resolve(
        global.partner_user_id.as_deref(),
        global.partner_user_secret.as_deref(),
        &ProcessEnv,
        &Keychain,
    )?;

    // The secret is never printed, only its length, which is enough to tell
    // "wrong secret" from "no secret".
    let view = View::new(
        "credentials",
        vec!["SOURCE", "PARTNER USER ID", "SECRET"],
        vec![vec![
            resolved.source.describe().to_owned(),
            resolved.partner_user_id.clone(),
            format!("{} characters", resolved.partner_user_secret.len()),
        ]],
        json!({
            "source": match resolved.source {
                Source::Flags => "flags",
                Source::Environment => "environment",
                Source::Keychain => "keychain",
            },
            "partner_user_id": resolved.partner_user_id,
            "secret_length": resolved.partner_user_secret.len(),
        }),
    );
    view.print(global.output)
}

fn logout(global: &GlobalArgs) -> Result<()> {
    let message = if Keychain.clear()? {
        "removed the stored credentials"
    } else {
        "nothing was stored"
    };
    if std::env::var_os(ENV_ID).is_some() || std::env::var_os(ENV_SECRET).is_some() {
        eprintln!("note: {ENV_ID}/{ENV_SECRET} are still set and take precedence");
    }
    View::acknowledgement("credentials", message).print(global.output)
}

fn prompt(label: &str) -> Result<String> {
    use std::io::Write as _;
    eprint!("{label}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading stdin")?;
    Ok(line.trim().to_owned())
}
