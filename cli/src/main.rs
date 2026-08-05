//! `expensify` — command-line client for the Expensify Integration Server.

mod auth;
mod cli;
mod commands;
mod error;
mod fingerprint;
mod observe;
mod output;
mod spec;
mod view;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> ExitCode {
    let parsed = cli::Cli::parse();
    init_logging(parsed.global.verbose, parsed.global.quiet);
    // Taken before the command consumes it: a failure has to be attributed to
    // a command even when the command is what went missing.
    let command = cli::path(&parsed.command);

    match commands::run(parsed).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => ExitCode::from(error::report(&err, command)),
    }
}

/// Two ceilings, not one: our own diagnostics have to outrank the transport's.
///
/// `hyper`/`h2` log frame handling at DEBUG, so a single `max_level` loud
/// enough to print request bodies buries them under frame noise — which is
/// what `-vv` used to do. Dependencies stay at WARN until `-vvv` asks.
fn init_logging(verbose: u8, quiet: bool) {
    let (ours, dependencies) = match (quiet, verbose) {
        (true, _) => (LevelFilter::ERROR, LevelFilter::OFF),
        (_, 0) => (LevelFilter::WARN, LevelFilter::OFF),
        (_, 1) => (LevelFilter::INFO, LevelFilter::WARN),
        (_, 2) => (LevelFilter::DEBUG, LevelFilter::WARN),
        (_, _) => (LevelFilter::TRACE, LevelFilter::DEBUG),
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time()
                .with_writer(std::io::stderr),
        )
        // The binary's crate name is `expensify`, so this covers everything
        // this repository emits.
        .with(
            Targets::new()
                .with_target("expensify", ours)
                .with_default(dependencies),
        )
        .init();
}
