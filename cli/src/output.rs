//! Result rendering: one `View` per command, printed as a table or as JSON.

use std::io::Write;

use anyhow::Result;
use clap::ValueEnum;
use comfy_table::{ContentArrangement, Table, presets};
use serde_json::Value;

/// Variants are deliberately undocumented: clap renders per-variant help as
/// a multi-line block on every subcommand that inherits `--output`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Wide,
    Json,
}

impl OutputFormat {
    /// `wide` differs from `table` only in which columns a command builds.
    pub fn is_wide(self) -> bool {
        self == Self::Wide
    }
}

/// A result set in both shapes. Commands build one and print it; the format
/// flag decides which half is used, so the JSON shape never drifts from what
/// a command can show.
pub struct View {
    noun: String,
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
    json: Value,
}

impl View {
    pub fn new(
        noun: impl Into<String>,
        headers: Vec<&'static str>,
        rows: Vec<Vec<String>>,
        json: Value,
    ) -> Self {
        Self {
            noun: noun.into(),
            headers,
            rows,
            json,
        }
    }

    /// A confirmation with nothing to tabulate — a write that succeeded.
    pub fn acknowledgement(noun: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            noun: noun.into(),
            headers: vec!["RESULT"],
            rows: vec![vec![message.clone()]],
            json: serde_json::json!({ "result": message }),
        }
    }

    pub fn print(&self, format: OutputFormat) -> Result<()> {
        let mut stdout = std::io::stdout().lock();
        match format {
            OutputFormat::Json => {
                serde_json::to_writer_pretty(&mut stdout, &self.json)?;
                writeln!(stdout)?;
            }
            OutputFormat::Table | OutputFormat::Wide if self.rows.is_empty() => {
                // Nothing on stdout, so `| wc -l` stays honest.
                eprintln!("No {} found.", self.noun);
            }
            OutputFormat::Table | OutputFormat::Wide => {
                writeln!(stdout, "{}", render_table(&self.headers, &self.rows))?;
            }
        }
        Ok(())
    }
}

/// Borderless, left-aligned, two spaces between columns.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = Table::new();
    table
        .load_preset(presets::NOTHING)
        .set_content_arrangement(ContentArrangement::Disabled)
        .set_header(headers.iter().map(|header| header.to_uppercase()));
    for row in rows {
        table.add_row(row.clone());
    }
    for column in table.column_iter_mut() {
        column.set_padding((0, 2));
    }
    table
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Some(true)` renders as `true`, absence as an empty cell rather than
/// `None`.
pub fn cell_opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_borderless_with_uppercase_headers() {
        let rendered = render_table(
            &["id", "name"],
            &[
                vec!["P1".into(), "Engineering".into()],
                vec!["P22".into(), "Ops".into()],
            ],
        );
        let lines: Vec<_> = rendered.lines().collect();
        assert_eq!(lines[0], "ID   NAME");
        assert_eq!(lines[1], "P1   Engineering");
        assert_eq!(lines[2], "P22  Ops");
        assert!(!rendered.contains('|'), "{rendered}");
    }

    #[test]
    fn empty_cells_are_blank_not_none() {
        assert_eq!(cell_opt(Option::<u8>::None), "");
        assert_eq!(cell_opt(Some(3)), "3");
    }
}
