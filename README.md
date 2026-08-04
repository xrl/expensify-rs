# expensify

Rust client for the [Expensify Integration Server API](https://integrations.expensify.com/Integration-Server/doc/).

Expensify exposes one endpoint that takes a JSON job description in a form
field. This crate turns that into typed operations where the compiler knows
which response shape comes back.

```rust
use expensify::{Client, Credentials, ExportTemplate, Json, ReportsQuery};

let client = Client::new(Credentials::new(partner_user_id, partner_user_secret));

let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed(TEMPLATE_SRC);

let file = client
    .export_reports(&template, ReportsQuery::since(date!(2026-07-01)))
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
`tax` field exists; not requesting them gives you one where reading `tax` is a
compile error.

Full rationale in [`docs/DESIGN.md`](docs/DESIGN.md). The approach is adapted
from Isabel Atkinson's RustConf 2024 talk on the MongoDB Rust driver — see
[`docs/prior-art.md`](docs/prior-art.md).

## Status

Pre-release. The type design is settled; the wire layer is in progress.

## License

MIT OR Apache-2.0, at your option.
