# expensify

[![CI](https://github.com/xrl/expensify-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/xrl/expensify-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/expensify.svg)](https://crates.io/crates/expensify)
[![docs.rs](https://docs.rs/expensify/badge.svg)](https://docs.rs/expensify)

Rust client for the [Expensify Integration Server API](https://integrations.expensify.com/Integration-Server/doc/).

```toml
[dependencies]
expensify = "0.2"
```

Expensify exposes one endpoint that takes a JSON job description in a form
field. This crate turns that into typed operations where the compiler knows
which response shape comes back.

```rust
use expensify::{Client, Credentials, ExportFormat, ExportTemplate, Json, ReportsQuery};
use serde::Deserialize;
use time::macros::date;

#[derive(Deserialize)]
struct ReportRow {
    report_id: String,
    total_cents: i64,
}

let client = Client::new(Credentials::new(partner_user_id, partner_user_secret));

// The template's type parameter decides what `download` gives you back.
let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed(TEMPLATE_SRC);

let file = client
    .export_reports(&template, ReportsQuery::since(date!(2026 - 07 - 01)))
    .format(ExportFormat::Json)   // the default is Csv for every template type
    .mark_as_exported("acme-etl")
    .await?;

let rows: Vec<ReportRow> = client.download(&file).await?;
```

## Design

Operations follow one shape: `client.verb_noun(required args)` returns an
action struct, optional settings are fluent setters, and `.await` executes it.
Options you don't set cost you nothing to ignore.

Where the API's response shape depends on a request flag, a type parameter
decides it instead of a boolean — so results don't arrive wrapped in `Option`
for you to `unwrap`. Requesting a policy's tax rates gives you a `Policy` whose
`tax` field exists; not requesting them makes reading `tax` a compile error. When
the selection is only known at run time — CLI flags, a config file —
`get_policies_dynamic` takes it as data and hands back `Option`s instead.

Full rationale in [`docs/DESIGN.md`](docs/DESIGN.md). The approach is adapted
from Isabel Atkinson's RustConf 2024 talk on the MongoDB Rust driver — see
[`docs/prior-art.md`](docs/prior-art.md).

## What you get

- **Rate limiting is on by default**, matching Expensify's published 5-per-10s
  and 20-per-60s budgets. Opt out with `Client::builder(..).no_rate_limiting()`
  if you're pacing requests yourself.
- **HTTP 200 does not mean success** in this API — the response body carries its
  own status code, and this crate maps it before handing you a result.
- **rustls only.** No OpenSSL, no system TLS.
- **`reqwest` is re-exported**, so the types in these signatures (`Url`,
  `reqwest::Client`, `StatusCode`) are nameable without a second dependency.
- MSRV 1.88.

One optional feature, `employee-updater-deprecated`, gates Expensify's
deprecated CSV employee updater. It is off by default and you almost certainly
want the Advanced Employee Updater instead.

## Command line

The `cli/` package in this workspace builds an `expensify` binary over the same
operations:

```console
$ cargo install --path cli
$ expensify auth login                      # stored in the OS keychain
$ expensify get policies
$ expensify get policy 1234ABCD --with-categories --with-tax -o json
$ expensify export reports --template month-end.ftl --since 2026-07-01
$ expensify download export_1234.csv -O july.csv
```

`expensify skill install` writes a [Claude Code](https://claude.com/claude-code)
agent skill for this CLI into `~/.claude/skills/expensify/` — `--project` for a
repository-local `.claude/skills`, `--print` for stdout. The skill is compiled
into the binary from [`cli/skill/SKILL.md`](cli/skill/SKILL.md), so installing
needs neither a checkout nor a network.

Credentials resolve from `--partner-user-id`/`--partner-user-secret`, then
`EXPENSIFY_PARTNER_USER_ID`/`EXPENSIFY_PARTNER_USER_SECRET`, then the keychain —
so CI keeps using environment variables without touching a keychain that isn't
there. `expensify completion <shell>` prints a completion script, and exit codes
are documented in `expensify --help` for scripts that branch on them.

## Status

Working and tested, but **the wire format has not been verified against a live
Expensify account.** Expensify publishes no OpenAPI spec, no schema, and no
changelog, so every field name and value type here is derived from their prose
documentation. The test suite pins this crate's reading of those docs — it
cannot tell you the reading is right.

Where a guess carries real consequences, the affected method says so in its own
rustdoc rather than burying it here. The current list is tracked in
[`docs/DESIGN.md` § Open questions](docs/DESIGN.md#open-questions); the ones
worth knowing before you reach for them:

- `ExportReportsAction::test_run` — the flag's encoding is inferred. If it is
  wrong, the dry run is not dry and `on_finish` actions fire.
- `ReimburseAction` — Expensify may report a partially-applied reimbursement as
  a plain success, in which case the strict path cannot detect it.
- Some operations are deliberately withheld rather than shipped half-known.
  Merging (rather than replacing) policy tags is one; PDF export is another.
  Both are additive to restore once confirmed.

If you have a partner credential pair, `cargo run --example tour` reads
`EXPENSIFY_PARTNER_USER_ID` and `EXPENSIFY_PARTNER_USER_SECRET` from the
environment and exercises the distinctive paths. Reports of what Expensify
actually returns are the most useful contribution this crate can receive.

## License

MIT OR Apache-2.0, at your option.
