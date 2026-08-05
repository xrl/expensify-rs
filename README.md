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
use expensify::{Client, Credentials, ExportTemplate, Json, ReportsQuery};
use serde::Deserialize;
use time::macros::date;

#[derive(Deserialize)]
struct ReportRow {
    report_id: String,
    total_cents: i64,
}

let client = Client::new(Credentials::new(partner_user_id, partner_user_secret));

// The template's type parameter decides what `download` gives you back —
// and what the export asks Expensify to render (`fileExtension: json` here).
let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed(TEMPLATE_SRC);

let file = client
    .export_reports(&template, ReportsQuery::since(date!(2026 - 07 - 01)))
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
- **Secrets are typed, not remembered.** Every secret is a `Secret<T>`, which
  redacts in `Debug`/`Display` and cannot be serialized; the wire layer is the
  only place that unwraps one, so a new secret-bearing field cannot leak
  through a derived `Debug`.
- **Observability on every call.** `Client::builder(..).observe(..)` reports
  the request as sent (credentials redacted) and the raw response — status,
  content-type, body — for diagnosing a wire mismatch without reaching for
  curl. Off by default, no extra dependency, and `Recorder` captures exchanges
  for turning live responses into fixtures. Response bodies contain personal
  data; treat what you capture accordingly.
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
are documented in `expensify --help` for scripts that branch on them. `-v`
logs one line per API call and `-vv` prints the request and response bodies —
including any personal data the response carries.

## Status

**Partly verified against a live account, mostly not.** Expensify publishes no
OpenAPI spec, no schema, and no changelog, so most field names and value types
here are derived from their prose documentation, and the test suite pins this
crate's *reading* of those docs rather than the docs' correctness. A dozen
response shapes have now been recorded off a real account and are replayed as
fixtures; five documented claims turned out to be wrong, including the export
submit response, which meant `export_reports` never worked before 0.3.0.

[`docs/DESIGN.md` § Verification status](docs/DESIGN.md#verification-status)
lists every response shape as observed, doc example, or inference — read it
before trusting anything not marked observed. Where a guess carries real
consequences the affected method says so in its own rustdoc; the ones worth
knowing before you reach for them:

- `ExportReportsAction::test_run` — the flag's encoding is inferred. If it is
  wrong, the dry run is not dry and `on_finish` actions fire.
- `DomainClient::reconcile` — its response shape is a doc example, and the
  exporter's turned out not to match its own. Confirming it needs a
  domain-admin credential.
- Some operations are deliberately withheld rather than shipped half-known.
  PDF export is one. Merging (rather than replacing) policy tags is not coming
  back: `action: "merge"` was observed deleting every unlisted tag.

If you have a partner credential pair, `cargo run --example tour` reads
`EXPENSIFY_PARTNER_USER_ID` and `EXPENSIFY_PARTNER_USER_SECRET` from the
environment and exercises the distinctive paths. Reports of what Expensify
actually returns are the most useful contribution this crate can receive.

## License

MIT OR Apache-2.0, at your option.
