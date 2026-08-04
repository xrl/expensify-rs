//! `expensify` — command-line client for the Expensify Integration Server.

mod auth;
mod cli;
mod commands;
mod error;
mod output;
mod spec;
mod view;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let parsed = cli::Cli::parse();
    init_logging(parsed.global.verbose, parsed.global.quiet);

    match commands::run(parsed).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error::report(&err);
            ExitCode::from(error::exit_code(&err))
        }
    }
}

/// Logging is CLI-side only: the library emits no `tracing` events.
fn init_logging(verbose: u8, quiet: bool) {
    let level = match (quiet, verbose) {
        (true, _) => LevelFilter::ERROR,
        (_, 0) => LevelFilter::WARN,
        (_, 1) => LevelFilter::INFO,
        (_, _) => LevelFilter::DEBUG,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}
