//! Misuse 14: conjuring a typed download from a bare filename.
//! `from_parts` lets you re-assert a file system for a name persisted
//! out-of-band, but only untyped — the decode type may only come from the
//! template that produced the file.

use expensify::{ExportedFile, FileSystem, Json};

struct Row;

fn main() {
    let _ = ExportedFile::<Json<Vec<Row>>>::from_parts("export_1.json", FileSystem::IntegrationServer);
}
