//! `expensify download` — fetch a file an export job produced.

use std::io::Write;

use anyhow::{Context, Result};
use expensify::{ExportedFile, FileSystem};

use crate::cli::{DownloadArgs, FileSystemArg, GlobalArgs};
use crate::commands::{client, note};

pub async fn run(args: DownloadArgs, global: &GlobalArgs) -> Result<()> {
    // The library pins the file system on the handle its export returned;
    // a filename that arrived out of band re-asserts it here.
    let file = ExportedFile::from_parts(
        &args.filename,
        match args.file_system {
            FileSystemArg::IntegrationServer => FileSystem::IntegrationServer,
            FileSystemArg::Reconciliation => FileSystem::Reconciliation,
        },
    );

    let client = client(global)?;
    let bytes = client
        .download(&file)
        .await
        .with_context(|| format!("downloading {}", args.filename))?;

    match &args.out {
        Some(path) => {
            std::fs::write(path, &bytes).with_context(|| format!("writing {path}"))?;
            note(global, format!("wrote {} bytes to {path}", bytes.len()));
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&bytes)?;
            stdout.flush()?;
        }
    }
    Ok(())
}
