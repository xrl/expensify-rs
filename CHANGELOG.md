# Changelog

## Unreleased

**Breaking — the next library release must be 0.3.0, not 0.2.1.** Four public
signatures change shape (below). Everything else here is additive.

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
