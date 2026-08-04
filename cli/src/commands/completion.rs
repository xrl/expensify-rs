//! `expensify completion` — shell completion scripts.

use anyhow::Result;
use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs};

pub fn run(args: CompletionArgs) -> Result<()> {
    let mut command = Cli::command();
    let name = command.get_name().to_owned();
    clap_complete::generate(args.shell, &mut command, name, &mut std::io::stdout());
    Ok(())
}
