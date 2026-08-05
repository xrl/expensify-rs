# Changelog

## Unreleased

Recommended: **expensify 0.4.0**. Pre-1.0 a minor bump is the breaking-change
signal, and this changes observable behaviour rather than adding to it — a
`Json<_>` export that relied on the CSV default now renders JSON. Nobody
plausibly relied on it (the pairing was already a decode failure at download),
but "nobody plausibly does" is not the same as additive, and the two added
trait bounds are a second, smaller reason.

### The export format now comes from the template's type

`ExportTemplate<Json<Vec<Row>>>` said the template emits JSON and the crate
sent `fileExtension: "csv"` anyway, so a caller who forgot
`.format(ExportFormat::Json)` got CSV bytes fed to a JSON decoder — and found
out at **download**, after the export had run server-side and any `onFinish`
had fired. `docs/DESIGN.md`'s own running example shipped without that call.

- `FromExport` gains `EXPORT_FORMAT: ExportFormat` and
  `RECONCILIATION_FORMAT: ReconciliationFormat`. `Json<T>` sets both to
  `Json`; `Bytes` and `String` keep `Csv`. Each export action reads the one
  for its job as its default.
- **Both consts are defaulted to `Csv`**, so existing user markers compile and
  behave unchanged. The trait stays open for them.
- **An explicit `.format()` still wins**, including where it contradicts the
  marker. That contradiction is the caller's to make; what changed is that
  the resulting decode failure now names it. `Client::download` compares the
  file's extension with the marker's format and, when they disagree, reports
  the override instead of `expected value at line 1 column 1` — which read as
  an accusation against the FreeMarker source. A decode error whose extension
  matches is passed through untouched.
- `Error::Decode` now renders its source (`decode error: json: …`) rather than
  the bare `decode error`, which is what the rest of the crate's docs already
  claimed it did.

**Breaking:** `Client::export_reports` and `DomainClient::reconcile` gain an
`F: FromExport` bound. Technically breaking, practically not — an
`ExportTemplate<F>` cannot be constructed without it, so only a downstream
generic passthrough would need the bound added.

Closes `docs/DESIGN.md` open question 5, which had rejected this as "a small
trait wart".

## expensify 0.3.0 / expensify-cli 0.2.0 — 2026-08-05

**Breaking.** Four public library signatures change shape (below). Everything
else here is additive.

Five of these fixes exist because the crate was finally run against a live
Expensify account. Every one was a documented behaviour that turned out to be
inferred rather than observed, and the whole test suite was green throughout —
the mocks asserted the same reading of the prose docs that the code did. The new
`docs/DESIGN.md` § Verification status records which response shapes are now
observed, which rest on a doc example, and which are still inference.

### Fixed against the live API

Five behaviours were probed against a real Expensify account and did not match
the documentation they were built from. `docs/DESIGN.md` § Verification status
now records, per response shape, whether it is observed, a doc example, or
inference; the recorded bodies are replayed in `tests/replay.rs`.

- **The Report Exporter answers a bare filename, not a JSON envelope**, so
  `export_reports` — the crate's flagship operation — failed every time with
  `expected value at line 1 column 1`. The documented
  `{"responseCode":200,"filename":…}` shape is reconciliation's and had been
  generalized. The exporter now accepts either, discriminating on the body's
  shape; content-type is not consulted, because this endpoint sends JSON as
  `text/plain` for some jobs and as `application/json` for others.
  Reconciliation's own shape remains unconfirmed.
- **Reimbursement reports partial success under `responseCode: 200`**, not
  207 — both when every report is skipped and when some succeed. Strict mode
  keyed on 207, so it returned `Ok` and discarded the skip reasons, breaking
  the guarantee it exists to make. It now fails whenever `skippedReports` or
  `failedReports` is non-empty, whatever the code. `tolerate_partial()` is
  unchanged.
- **`employeeEmail` is required when creating expenses.** Breaking:
  `Client::create_expenses(employee_email, expenses)` takes it as an argument
  and `CreateExpensesAction::employee_email` is gone. It does not default to
  the credential owner; without it Expensify answers 410. CLI:
  `create expenses --employee-email` is now required.
- **Report creation needs no unlock from Expensify support** — documented as
  requiring one, it worked on a policy-admin trial account. Documentation
  only; the claim is now stated as unconfirmed rather than asserted.
- **Tag "merge" is destructive**, confirmed: sending one tag with
  `action: "merge"` deleted the two unlisted ones and answered
  `{"responseCode":200}`. `TagsUpdate` was already replace-only on suspicion;
  that decision is now permanent and has a compile-fail case.

Also observed: the expense-rules creator returns no rule ID (so `()` is
right), the undocumented `responseCode: 666` is the only way to learn a rule's
ID, and `REIMBURSED` really is the only accepted report status.

- **`CreatedTransaction::report_id: Option<ReportId>`** — new field, recording
  an undocumented side effect: an expense created without `Expense::report_id`
  is not left loose. Expensify opens a report for it and names it in the
  response, which this crate was discarding, so a caller could not find their
  own expense without a separate export. The CLI prints it as a `REPORT ID`
  column and a `report_id` key. `Option` even though every observed response
  carries it: the expense exists by the time the response is decoded, so a
  missing key must not turn a created expense into an error nobody can act on
  or safely retry. Technically breaking for anyone destructuring or
  struct-literal-constructing `CreatedTransaction`, which 0.3.0 already is.

### Everything else

- `Secret<T>` and `MaskedUrl`: redaction is now carried by the field's type
  rather than by a hand-written `Debug` on each holder. `Debug`/`Display`
  redact, `expose()` is the only read path, and there is no `Serialize` impl —
  so a secret can only reach the wire through the job builder, which stores it
  out-of-band and substitutes it in at render time. Breaking:
  - `Credentials::new`'s second parameter is `impl Into<Secret<String>>`
    (`&str` and `String` still work; an `Into<String>` type that is not
    `Into<Secret<String>>` no longer does).
  - `SftpConnection::password` is `Secret<String>` (public field).
  - `EmployeeSource::FetchUrl`'s `password` is `Option<Secret<String>>` and
    its `url` is `MaskedUrl` (public variant fields). `"…".into()` covers both.
- `ClientBuilder::observe(impl Observer)` reports every request and response:
  the request body as sent with credentials redacted, and the response status,
  content-type and raw body. Off by default. `Recorder` captures exchanges in
  memory, which is the basis for recording live responses as test fixtures.
  No new dependency. **Observed response bodies contain personal data.**
- CLI: `-v` logs one line per API call; `-vv` prints the full request and
  response bodies and warns that responses carry personal data; `-vvv` adds
  transport tracing. Dependency logging is filtered by target, so `-vv` no
  longer buries the exchange under `h2` frame noise.
- CLI: `expensify skill install` writes the Claude Code agent skill embedded in
  the binary (`cli/skill/SKILL.md`) into a personal or repository-local skills
  directory. The library is unchanged.
- Skill: a section on diagnosing a failed command — which commands are safe to
  re-run under `-vv` and which have already had their server-side effect, and
  how to file the defect without republishing a credential or an employee's
  data. The library is unchanged.
- CLI: the OS keychain is read under a timeout. Keychain access is granted per
  executable, so a freshly built binary raises a permission prompt on its first
  read; with nothing attached to answer it the process blocked forever printing
  nothing at all. Bounded now at 120s with a terminal attached and 10s without,
  with the wait itself announced after one second, and a failure that names
  every way out. `EXPENSIFY_KEYCHAIN_TIMEOUT_SECS` overrides it; `0` restores
  the old unbounded wait. The library is unchanged.
- CLI: failures print the account that was authenticated as and, where the
  client cannot account for what happened, a stable defect fingerprint
  (`EXP-XXXXXXXX`) derived from the command, the exit code and the error's
  discriminant. Neither the version nor any message text feeds it, so one
  defect keeps one token across releases and issues can be deduplicated by
  searching for it exactly. Failures Expensify or the CLI already explained
  carry no fingerprint. `-vv` now names the account whose data it is about to
  print. The library is unchanged; no new dependencies.

## 0.2.0 — 2026-08-04

Three additions for callers driving the API at run time rather than from
source. All additive; nothing in 0.1.0 changes shape.

- `FetchState::project` inverts the `Wrap` GAT — `Fetched` yields `Some`,
  `Omitted` yields `None` — so code generic over the fetch states can read a
  slot instead of re-deriving the trait locally. `Policy::project` applies it
  to all five sections at once.
- `Client::get_policies_dynamic(ids, fields)` takes the field selection as
  data (`PolicyField`) and answers `DynamicPolicy`, whose sections are
  `Option`s. This is an escape hatch, documented as one: it reintroduces the
  `unwrap` the typestate exists to remove, and `get_policies` remains the
  default. Both getters share one request path.
- `reqwest` is re-exported as `expensify::reqwest`, plus `expensify::Url`.
  Naming `ClientBuilder::base_url`'s argument — or `http_client`'s, or the
  types inside `Error::Transport` and `Error::Http` — no longer needs a
  second dependency.

## 0.1.0 — 2026-08-04

First release. Covers the Expensify Integration Server's export, download,
reconciliation, policy read/write, expense and report creation, reimbursement,
expense rules, tag approvers, and employee updater jobs.

Wire shapes are derived from Expensify's prose documentation and have not been
verified against a live account — see `docs/DESIGN.md` § Open questions, and the
rustdoc on individual methods where a guess carries consequences.

Deliberately withheld until a live probe confirms behavior; restoring each is
additive:

- Merging (rather than replacing) policy tags. Expensify's prose says a tags
  update replaces, and the inline parameter table documents no `action` key, so
  a method named `merge_*` could silently delete every unlisted tag.
- PDF export. Expensify emits one PDF per report, which a single `ExportedFile`
  handle cannot name.
