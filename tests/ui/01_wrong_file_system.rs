//! Misuse 1: pairing a reconciliation filename with the default
//! `integrationServer` file system. `ExportedFile`'s fields are private and
//! `download()` has no file-system parameter, so there is no spelling for it.

use expensify::{ExportedFile, FileSystem};

fn main() {
    let _: ExportedFile = ExportedFile {
        name: "is_reconciliation_5429137734434770049.csv".into(),
        file_system: FileSystem::IntegrationServer,
    };
}
