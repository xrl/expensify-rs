# expensify-rs design

Type-system design for a Rust client for the Expensify Integration Server
API. `src/` is signature-authoritative — every public type, bound, and
`IntoFuture` impl below exists there and passes `cargo check` (both default
and `employee-updater-deprecated` feature sets). `examples/tour.rs`
compiles the running example; every entry in
[§ Misuses](#misuses-made-uncompilable) has a `tests/ui/` case verified to
fail compilation with the quoted error class.

Source of truth: <https://integrations.expensify.com/Integration-Server/doc/>
and `doc/employeeUpdater/`, read 2026-08-04. No OpenAPI spec exists (the
`openapi.json`/`swagger.json` paths are soft-404s). No versioning or
deprecation signal from Expensify; treat the wire layer as the part most
likely to need maintenance and keep it in one module (`wire.rs`).

A dozen response shapes have since been probed against a live account, and
five documented claims turned out to be wrong — including the flagship export, which had
never worked. [§ Verification status](#verification-status) says which shapes
are observed, which are doc examples, and which are still inference; read it
before trusting anything below that is not marked observed.

Corrections/refinements to the pre-read map, verified against the docs:

- Exporter `inputSettings.filters` is required and needs at least one of
  `reportIDList` / `startDate` / `approvedAfter`; `endDate` becomes
  required when the start anchor is >1 year old; span ≤ 1 year.
- Exporter has a top-level `test` flag (skips `onFinish`); `onReceive`
  only supports `{"immediateResponse":["returnRandomFileName"]}`.
- Reconciliation is synchronous-only (`async: false` is the only
  supported value) — so its output file is immediately downloadable,
  unlike report exports.
- Report Status Updater: only `REIMBURSED`, only from Approved; `filters`
  needs `reportIDList` or `startDate`; 207 partial responses carry
  `reportIDs` + `skippedReports` + `failedReports` (`{reportID, reason}`).
- Advanced Employee Updater is ordinary urlencoded (`data` form field);
  only the *deprecated* employee updater is `multipart/form-data`.
- Rate limits on the main doc page: 5 req/10 s and 20 req/60 s → 429. The
  "50 jobs started per minute" figure did not appear on the pages read;
  design assumes the stricter pair.
- Doc typo: policy-updater tag `fileType` is listed as `"cvs"`/`"tsv"`;
  the example JSON uses `"csv"`. Send `"csv"`.
- The Policy Getter answers `tags` in **two** shapes on the same doc page:
  a flat tag list, and a list of tag *levels* each wrapping its own
  `tags`. Both are modelled (`PolicyTags`), because guessing one makes the
  other a hard decode failure for the whole multi-policy response.
- `test` is typed `String` in the exporter parameter table, not boolean.

## Running example

Month-end close: export July's approved reports (typed by the caller's
template), download them, book an expense, reimburse the exported
reports, and read back policy tax rates — no `unwrap` anywhere.
Full version: `examples/tour.rs`.

```rust
#[derive(Deserialize)]
struct ReportRow { report_id: ReportId, employee: String, total_cents: i64 }

let client = Client::new(Credentials::new(id, secret));

// Template carries the output type. TEMPLATE_SRC is FreeMarker emitting JSON.
let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed(TEMPLATE_SRC);

let file = client
    .export_reports(&template,
        ReportsQuery::since(date!(2026-07-01)).until(date!(2026-08-01))
            .policy_ids([&policy])
            .not_yet_exported_as("acme-etl"))
    .state(ReportState::Approved)
    .format(ExportFormat::Json)                // required: default is Csv for every marker
    .mark_as_exported("acme-etl")
    .await?;                                   // -> ExportedFile<Json<Vec<ReportRow>>>

let rows: Vec<ReportRow> = client.download(&file).await?;   // typed by the file handle

client.create_expenses("ap@acme.com", [       // required: there is no default
    Expense::new("Cloud Hosting Inc", date!(2026-07-31), Money::new(129_00, "USD"))
        .category("Infrastructure")
        .external_id("hosting-2026-07"),
]).await?;

let outcome = client
    .mark_reports_reimbursed(ReimburseTargets::report_ids(rows.iter().map(|r| &r.report_id)))
    .payment_source("ACME-AP")
    .tolerate_partial()
    .await?;                                   // -> ReimburseOutcome (207 is Ok here)

let policies = client.get_policies([&policy]).with_tax().with_categories().await?;
let info = &policies[&policy];
for cat in &info.categories { /* Vec<Category>, not Option */ }
if let Some(tax) = &info.tax { /* Option is data-dependent, not request-dependent */ }
```

## Design rules

Following the MongoDB bulk-write talk (Atkinson, RustConf 2024):

1. **Continuity** — one pattern everywhere: `client.verb_noun(required args)`
   → action struct → fluent optional setters → `.await` via `IntoFuture`.
   "Required args in the constructor, optionals as setters" also governs
   data builders (`Expense::new(merchant, date, amount)` + setters).
2. **Options cost nothing when unused** — no options structs in any
   signature, no `Option<Opts>` parameters. Boolean options whose only
   useful value is non-default drop the parameter (`.dry_run()`,
   `.test_run()`, `.admin_only()`, `.tolerate_partial()`).
3. **Best possible type information** — where a runtime flag decides the
   response shape, a type parameter decides it instead, so the
   `Output` of `IntoFuture` is exact and unwrap-free.

Extra rule this crate adds: a phantom/sealed mechanism must make a
statable misuse unrepresentable; capability facts the library cannot
verify (domain-admin-ness, support-enabled report creation) stay runtime
errors — see [§ Rejected mechanisms](#rejected-mechanisms).

## Wire model (implementer contract)

One endpoint: `POST https://integrations.expensify.com/Integration-Server/ExpensifyIntegrations`,
`application/x-www-form-urlencoded` (exception: deprecated employee
updater is `multipart/form-data`). Form fields:

| field | when |
|---|---|
| `requestJobDescription` | always — JSON: `type`, `credentials{partnerUserID,partnerUserSecret}`, `inputSettings`, plus job-specific `onReceive`/`outputSettings`/`onFinish`/`test` and top-level `categories`/`tags`/`reportFields`/`tagApprovers` |
| `template` | export + reconciliation jobs (FreeMarker source) |
| `file` | policy-updater tag CSV/TSV |
| `data` | employee feeds (advanced: urlencoded; deprecated: multipart) |

Responses are JSON with `responseCode` (and `responseMessage` on error)
**even under HTTP 200** — body code wins. **Two jobs are exceptions**, and
both discriminate on the shape of the body rather than on any header:

- the Downloader, whose success body is the raw file — non-200 HTTP or a JSON
  error envelope is failure, everything else is file content;
- the Report Exporter, whose success body is the bare generated filename
  (`export0fd99e06-….csv`, no JSON at all). A JSON object is an envelope —
  an error, or the documented shape if it ever arrives — and anything else is
  the name, accepted only if it looks like one (non-empty, single line).

**Content-type is not a discriminator anywhere.** The same endpoint answers
JSON as `text/plain;charset=utf-8` (expense rules, reimburse) and as
`application/json` (policy creator), and answers the exporter's bare filename
as `text/plain;charset=utf-8` too. Keying on it would have looked like it
worked. All of this lives in `wire.rs` (private); public types carry serde
attrs only where the mapping is 1:1.

Wire mapping notes: all JSON keys camelCase (`#[serde(rename_all)]` or
explicit renames); amounts are integer cents; `reportState` is a
comma-joined list; exporter `limit` and `test` serialize as strings;
report-field keys are normalized (non-alphanumeric → `_`) client-side
before sending; `Employee` feed serializes to the documented JSON array;
`TagApprover::clear` serializes `approver: ""`.

Deserialization rules the open vocabulary imposes: every enum whose values
Expensify controls (`PolicyPlan`, `PolicyRole`, `ReportFieldType`) carries
an `Other(String)` catch-all, because one policy on an unmodelled plan must
not fail an entire `list_policies()`. Enums this crate *sends* stay closed.
Optional-on-the-wire booleans (`enabled`) default rather than fail. A blank
string is absence; a *non-blank* value that will not parse is
`DecodeError`, never a silent `None`.

## Client, credentials, domain scope

```rust
pub struct Credentials { partner_user_id: String, partner_user_secret: Secret<String> }
impl Credentials {
    pub fn new(id: impl Into<String>, secret: impl Into<Secret<String>>) -> Self;
    pub fn partner_user_id(&self) -> &str;      // not a secret; it names the integration
}
// derives: Clone, Debug — the derive is safe because the field type redacts.
// No Serialize. See § Secrets.

#[derive(Clone)] pub struct Client { inner: Arc<ClientInner> }   // cheap clone; actions own one
// ClientInner (private): reqwest::Client, Credentials, reqwest::Url,
//                        Option<RateGate>, Option<Arc<dyn Observer>>

impl Client {
    pub fn new(credentials: Credentials) -> Self;                // prod endpoint, limiter on
    pub fn builder(credentials: Credentials) -> ClientBuilder;
}

pub struct ClientBuilder;   // base_url(Url), http_client(reqwest::Client),
                            // no_rate_limiting(), observe(impl Observer),
                            // build() -> Client
```

`reqwest` is re-exported (`expensify::reqwest`, plus `expensify::Url` for the
one type that appears in a signature callers write). Four reqwest types are
in this crate's public API — `Url`, `reqwest::Client`, `reqwest::Error`
inside `Error::Transport`, `StatusCode` inside `Error::Http` — and none of
them were nameable without a second dependency, which would also be a second
*version*: a `Url` from a differently-versioned `url` crate is a different
type and would not compile. See [§ Rejected mechanisms](#rejected-mechanisms)
for why `base_url` still takes a parsed `Url` rather than `impl TryInto<Url>`.

`Client::domain(name) -> DomainClient` scopes the two operations whose
jobs require a `domain` input (reconciliation, domain card list):

```rust
#[derive(Clone, Debug)] pub struct DomainClient { client: Client, domain: String }
impl DomainClient {
    pub fn name(&self) -> &str;
    pub fn reconcile<F>(&self, template: &ReconciliationTemplate<F>,
                        start: Date, end: Date, scope: ReconciliationScope) -> ReconcileAction<F>;
    pub fn card_list(&self) -> DomainCardListAction;
}
```

This is a *data* scope (the jobs need the domain string), not a
capability claim — Expensify still 403s non-domain-admin credentials.

## Secrets

Four values in this API are secrets, and they were previously protected by
hand-written `Debug` impls on the three types that hold them. That is
discipline: it is correct exactly as long as everyone adding a field
remembers, and the failure is silent. Two types replace it.

```rust
pub struct Secret<T = String>(T);          // Clone, PartialEq, Eq
impl<T> Secret<T> {
    pub fn new(value: T) -> Self;
    pub fn expose(&self) -> &T;            // the only read path
    pub fn into_inner(self) -> T;
}
impl<T> From<T> for Secret<T>;  impl From<&str> for Secret<String>;
// Debug and Display both render `<redacted>`. NO Serialize, NO Deref.

pub struct MaskedUrl(String);              // Clone, PartialEq, Eq
impl MaskedUrl {
    pub fn new(url: impl Into<String>) -> Self;
    pub fn expose(&self) -> &str;          // whole, userinfo included
    pub fn masked(&self) -> String;        // `https://<redacted>@host/path`
}
// From<&str>/<String>/<&Url>. Debug and Display render the masked form.
```

Applied to: `Credentials::partner_user_secret`, `SftpConnection::password`,
`EmployeeSource::FetchUrl::password` (all `Secret<String>`),
`EmployeeSource::FetchUrl::url` (`MaskedUrl`). `Client`'s `base_url` stays a
`Url` — reqwest needs one — and is printed through `MaskedUrl`.

**Why userinfo is not `Secret<Url>`.** It was the obvious unification and it
is wrong: `Secret` redacts the whole value, and for a URL the host and path
are the half you print a URL *for* — deleting them turns "the feed at
hr.acme.com 404s" into "a feed 404s". Two types, one rule: redacted by
default in every human-facing rendering, raw only through a named accessor.

**Why `Secret` has no `Serialize`.** Redaction is for humans and the wire
needs the real value, so the two must not be the same switch. Absent a
`Serialize` impl, `json!(secret)` does not compile, and the *only* way a
secret reaches the job description is `JobRequest::secret`, which stores it
out-of-band and puts an opaque placeholder in the JSON tree. Substitution
happens once per rendering: real values for the outgoing body, `<redacted>`
(or `MaskedUrl::masked`) for the observed one. So the observable body is not
a filtered copy of the real one — it is rendered from a tree that has never
held a secret, and a new secret-bearing field has to route through the same
door to compile at all.

What a new field must now do wrong to leak: bypass `Secret`, or call
`.expose()` and hand the result to `json!` directly. Both are visible in
review; `tests/secrets.rs` asserts both halves (nothing public prints a
sentinel; the wire body still carries it) over the public API.

## Observability

Off by default, one hook, every job:

```rust
pub trait Observer: Send + Sync + 'static {
    fn on_request(&self, request: &ObservedRequest) {}   // default no-op
    fn on_exchange(&self, exchange: &Exchange);
}
impl<F: Fn(&Exchange) + Send + Sync + 'static> Observer for F {}   // closures

pub struct ObservedRequest;   // url() -> &MaskedUrl, job_type(), job_description(),
                              // fields() -> impl Iterator<Item = (&str, &str)>, field(name)
pub struct Exchange;          // request(), status(), content_type(), body() -> &Bytes,
                              // body_text() -> Cow<str>, duration()
pub struct Recorder;          // Observer + Clone; exchanges(), take(), len(), is_empty()
// Both request types: Clone, Debug, Display. Display is the CLI's rendering.

impl ClientBuilder { pub fn observe(self, observer: impl Observer) -> Self }
```

`ClientInner` holds `Option<Arc<dyn Observer>>`; `None` skips the redacted
rendering, the timer and the body clone, so the unused cost is one `Option`
check per request. The hook sits in `Client::raw`, the one function both
`send` and `send_download` go through — there is no per-operation opt-in to
forget.

Two callbacks rather than one, because a request that never answers is
exactly when you want to see it: `on_request` fires before the send, so a
connection failure still shows what went out (it then surfaces as
`Error::Transport`, and there is no exchange to report). `Exchange` therefore
needs no `Option<StatusCode>`.

**Dependency argument: no new dependency.** `tracing` is the idiomatic choice
and is nearly free without a subscriber, but it cannot do the thing this
feature exists for twice over. Capturing exchanges through `tracing` means
writing a `Subscriber`, rendering typed data into formatted fields and
parsing it back — structure to string to structure — where a callback hands
over the bytes. And the library would gain a semver-relevant dependency to
serve a decision (what a log line is) that belongs to the binary. The CLI,
which already runs a subscriber, bridges the two in one 25-line file
(`cli/src/observe.rs`). The reverse arrangement cannot produce fixtures.

**Fixture recording**, concretely: install a `Recorder`, run the real call
once against live credentials, then for each exchange write
`exchange.body()` to `tests/fixtures/<job_type>-<case>.json` and keep
`status()` and `content_type()` beside it. A test replays those bytes through
a mock server — `tests/observe.rs::a_recorded_body_replays_as_a_fixture` does
exactly this round trip — so the assertion is against what Expensify said,
not against what this crate inferred it would say. That is the difference
that matters for a wire layer built from prose docs.

**Response bodies carry personal data** — employee names, emails, manager
chains, masked card numbers. Stated on `Observer`, on `ClientBuilder::observe`,
in the `observe` module docs, in the CLI's `--help`, and printed to stderr by
the CLI whenever `-vv` is on, because the realistic failure is a verbose log
pasted into a ticket.

## The action pattern

Every operation method returns a `#[must_use]` action struct holding a
`Client` clone plus owned inputs (templates are copied into the action so
futures are `'static`). Setters take `mut self` → `Self` (or a new type
parameterization, for typestate transitions). Execution is exclusively:

```rust
impl IntoFuture for XAction {
    type Output = Result<T, Error>;
    type IntoFuture = BoxFuture<Self::Output>;   // Pin<Box<dyn Future<Output=..> + Send + 'static>>
    fn into_future(self) -> Self::IntoFuture;
}
```

`BoxFuture` is a private alias; no `futures` dependency. All actions and
outputs are `Send` (phantoms use `PhantomData<fn() -> T>`, which is
`Send + Sync` regardless of `T`).

## Exports: templates, files, download

The sharpest constraint in the API: export output shape is defined by the
caller's FreeMarker template, and the Downloader's `fileSystem` must
match the job that produced the filename. Both are threaded through one
phantom chain: **template → exported file → download result**.

```rust
pub trait FromExport {                       // open for user impls (e.g. a Csv marker)
    type Output: Send + 'static;
    fn from_export(bytes: Bytes) -> Result<Self::Output, DecodeError>;
}
impl FromExport for Bytes  { type Output = Bytes; }    // escape hatch: raw bytes
impl FromExport for String { type Output = String; }   // UTF-8
pub struct Json<T>(PhantomData<fn() -> T>);            // marker, never instantiated
impl<T: DeserializeOwned + Send + 'static> FromExport for Json<T> { type Output = T; }
```

`FromExport` lives on marker types so `Output` can differ from `Self` —
`ExportedFile<Json<Vec<Row>>>` downloads to `Vec<Row>`, not
`Json<Vec<Row>>`. It is deliberately **not sealed**: implementing it on a
caller-side marker (CSV, XML) is the extensibility story.

```rust
pub struct ExportTemplate<F = Bytes>         { source: String, _out: PhantomData<fn() -> F> }
pub struct ReconciliationTemplate<F = Bytes> { /* same shape */ }
impl $T<Bytes>          { pub fn new(source: impl Into<String>) -> Self }   // untyped
impl<F: FromExport> $T<F> { pub fn typed(source: impl Into<String>) -> Self }
// both: source() -> &str; manual Clone/Debug (derives would demand F: Clone/Debug)
```

Two distinct template types because the two FreeMarker dialects evaluate
against disjoint data models (`reports` vs `cards→reports→transactionList`);
cross-wiring one into the other job is a compile error, not garbage output.
The library cannot check template *content* — the type records the
caller's declared intent at the only point they think about it.

```rust
#[derive(Clone, Copy, Serialize, Deserialize)]             // plain data; no phantom, no bounds
pub enum FileSystem { IntegrationServer, Reconciliation }  // renames: integrationServer/reconciliation

#[derive(Serialize, Deserialize)] #[serde(bound = "")]  // the phantom needs no bounds on F
pub struct ExportedFile<F = Bytes> { name: String, file_system: FileSystem, _out: PhantomData<fn() -> F> }
impl<F> ExportedFile<F> {
    pub(crate) fn from_response(name: String, fs: FileSystem) -> Self;  // ONLY normal constructor
    pub fn name(&self) -> &str;
    pub fn file_system(&self) -> FileSystem;
    pub fn untyped(&self) -> ExportedFile<Bytes>;
}
impl ExportedFile<Bytes> {
    pub fn from_parts(name: impl Into<String>, fs: FileSystem) -> Self;  // escape hatch, untyped only
}
```

The `fileSystem`/producer coupling is enforced by **non-constructibility,
not typestate**: fields are private, `export_reports` bakes in
`IntegrationServer`, `reconcile` bakes in `Reconciliation`, and the
download path never asks for a file system. Persistence story:
`ExportedFile` is `Serialize`/`Deserialize` (the file system rides
along, and `#[serde(bound = "")]` keeps `F` unconstrained), so a
serde round trip preserves both guarantees. For filenames stored as bare
strings out-of-band, `from_parts` exists but is restricted to
`ExportedFile<Bytes>`.

Scope of that restriction, stated precisely: `from_parts` is the only
*constructor* on the typed form, and it is absent there. `Deserialize` is
not — `#[serde(bound = "")]` is what makes the typed round trip work, and
the same impl accepts a hand-written `{"name":..,"file_system":..}`, so a
determined caller can mint an `ExportedFile<Json<Row>>` from a bare string
through serde. That is the accepted cost of the persistence story: the
guarantee is "you will not do this by accident", not "you cannot do this".

```rust
impl Client {
    pub fn export_reports<F>(&self, template: &ExportTemplate<F>, query: ReportsQuery)
        -> ExportReportsAction<F>;
    pub fn download<F: FromExport>(&self, file: &ExportedFile<F>) -> DownloadAction<F>;
}
impl<F: 'static> IntoFuture for ExportReportsAction<F> { type Output = Result<ExportedFile<F>, Error>; }
impl<F: 'static> IntoFuture for ReconcileAction<F>     { type Output = Result<ExportedFile<F>, Error>; }
impl<F: FromExport> IntoFuture for DownloadAction<F>   { type Output = Result<F::Output, Error>; }
```

Awaiting `export_reports` submits with
`onReceive.immediateResponse: ["returnRandomFileName"]` and resolves to
the handle; rendering continues server-side (poll by downloading —
no ready-signal exists, see Open questions). Reconciliation is
synchronous, so its handle is immediately downloadable.

The two answer that handle **differently**, which is the correction that made
`export_reports` work at all. The exporter sends the filename as a bare body;
reconciliation's documented `{"responseCode":200,"filename":…}` is what this
crate had generalized to both. Reconciliation's own shape is still
unconfirmed — probing it needs a domain-admin credential nobody here has — so
it keeps the documented envelope, now known not to be a shape the exporter
shares.

`ReportsQuery` (anchored constructors: there is no way to *spell* an empty
filter set — a documented 410 — though an empty iterator can still produce
one, see [§ Misuses](#misuses-made-uncompilable) entry 7):

```rust
pub struct ReportsQuery;   // Clone, Debug; all fields private
impl ReportsQuery {
    pub fn report_ids(impl IntoIterator<Item: Into<ReportId>>) -> Self;   // anchors
    pub fn since(Date) -> Self;
    pub fn approved_after(Date) -> Self;
    pub fn until(self, Date) -> Self;                                     // fluent extras
    pub fn policy_ids(self, impl IntoIterator<Item: Into<PolicyId>>) -> Self;
    pub fn not_yet_exported_as(self, impl Into<String>) -> Self;          // filters.markedAsExported
}
```

`ExportReportsAction<F>` setters: `.state(ReportState)` (repeatable →
comma-joined), `.limit(u32)`, `.employee_email(..)` (doc: restricted),
`.format(ExportFormat)`, `.file_basename(..)`,
`.on_finish(impl Into<OnFinish>)` (repeatable),
`.mark_as_exported(label)` (sugar for the common `OnFinish`),
`.test_run()`. Default format is `Csv` for every template marker,
including `Json<_>` — deriving it from `F` would need an associated const
on `FromExport`, which open question 5 rules on. Until then a
`Json<_>` template must call `.format(ExportFormat::Json)`; the mismatch
surfaces as a decode error, not silent corruption.

`OnFinish` constructors: `mark_as_exported(label)`,
`email(recipients) -> EmailOnFinish` (`.message(text)`, `Into<OnFinish>`),
`sftp_upload(SftpConnection)`. `message` lives on its own type because it
is meaningful for no other action; on the shared `OnFinish` it was a setter
that compiled and did nothing (misuse 17).
`SftpConnection { host, login, password: Secret<String>, port: u16 }`
(Clone, Debug; public fields; shared with the employee updater's SFTP
source). The password is a `Secret` rather than a hand-written `Debug` —
same rule as `Credentials`, and it matters more here because
`SftpConnection` is reachable from the `Debug` of `OnFinish`, of every export
action, and of `EmployeeSource`.

`ReconcileAction<F>` (`DomainClient::reconcile`): required args carry
`start`, `end`, `ReconciliationScope::{Unreported, All}`; setters
`.feed(name)` (default: all feeds), `.format(ReconciliationFormat)` — a
separate four-variant enum, so the formats this job rejects are
unrepresentable rather than server-rejected (misuse 18) —
`.email_on_finish(recipients)`. `async` is not
exposed — only `false` is supported upstream, so there is no parameter
(rule 3 of the talk: delete parameters with one useful value).

## Policy getter: fetch-flags as typestate

The Policy Getter's `fields` list decides which response sections exist.
Runtime modeling would be five `Option`s and an `unwrap` at every use
site — the exact shape `verbose_results()` fixed, times five. (That shape
does exist, as a deliberate opt-out for callers whose selection is not known
until run time — see
[§ The dynamic escape hatch](#the-dynamic-escape-hatch-and-what-it-costs).)

```rust
pub trait FetchState: sealed + Send + Sync + 'static {
    type Wrap<T: Payload>: Payload;               // GAT
    fn project<T: Payload>(wrapped: Self::Wrap<T>) -> Option<T>;   // the inverse of Wrap
    #[doc(hidden)] fn extract<T: DeserializeOwned + Payload>(
        field: &'static str, value: Option<serde_json::Value>) -> Result<Self::Wrap<T>, Error>;
}
pub struct Fetched;  impl FetchState for Fetched { type Wrap<T: Payload> = T; }
pub struct Omitted;  impl FetchState for Omitted { type Wrap<T: Payload> = NotFetched; }
pub struct NotFetched;                            // inert placeholder, no data, no methods
pub trait Payload: Debug + Clone + Send + Sync + 'static {}   // blanket impl
```

`Payload` bounds on the GAT let `Policy` derive `Debug`/`Clone` without
per-field where-clauses. `extract` is the deserialization hook so one
generic `IntoFuture` impl serves all 32 states. `FetchState` is sealed:
a third state has no meaning and would break `extract`'s contract.

`project` is `Wrap`'s inverse, and exists because `Wrap` alone is a one-way
door: code generic over the states can hold a `Wrap<T>` and has no way to
look inside, so every dynamic consumer was re-deriving this trait locally
(the CLI had one, and could only write it because both states and the GAT
are public). Adding it to the trait is not a new capability — it is the
capability callers already had, spelled once. It cannot weaken the static
path: nothing in `Policy`'s fields changes, and `Omitted::project` can only
answer `None`.

```rust
#[derive(Clone, Debug)]
pub struct Policy<Cats = Omitted, Fields = Omitted, Tags = Omitted, Tax = Omitted, Emps = Omitted>
where /* all: FetchState */ {
    pub categories:    Cats::Wrap<Vec<Category>>,
    pub report_fields: Fields::Wrap<Vec<ReportField>>,
    pub tags:          Tags::Wrap<PolicyTags>,        // Flat | Levels — see below
    pub tax:           Tax::Wrap<Option<TaxConfig>>,   // Option = "policy has no tax config"
    pub employees:     Emps::Wrap<Vec<PolicyEmployee>>,
}
pub type Policies<..> = HashMap<PolicyId, Policy<..>>;
```

The `Option` inside `tax` is deliberate: it encodes data-dependent
absence (`"tax": {}` on the wire), not request-dependent absence — the
distinction this whole mechanism exists to draw.

`tags` is an enum for the same reason it is not an `Option`: Expensify's
own documented sample answers `tags` flat for one policy and level-wrapped
for another, so the shape is genuine data, not a knob. Forcing one shape
would make the other a `DecodeError` for the *entire* `policyInfo` map —
one level-wrapped policy discarding every other policy's data. `PolicyTags`
exposes both variants plus `.tags()` for callers who do not care about
levels.

Two-stage builder enforces the API's "at least one field" requirement:

```rust
impl Client { pub fn get_policies(&self, ids: impl IntoIterator<Item: Into<PolicyId>>) -> GetPoliciesBuilder }

pub struct GetPoliciesBuilder;    // NOT IntoFuture. on_behalf_of(email);
                                  // with_categories/_report_fields/_tags/_tax/_employees
                                  //   -> GetPoliciesAction<..one Fetched..>

pub struct GetPoliciesAction<Cats, Fields, Tags, Tax, Emps>;  // defaults Omitted; #[must_use]
// each with_* exists ONLY on the impl block where its own slot is Omitted
// (double-select is a method-not-found error); on_behalf_of on all states.
// Runtime `fields: Vec<&'static str>` accumulates the wire list; the type
// parameters shape only the response.

impl<..all FetchState..> IntoFuture for GetPoliciesAction<Cats, Fields, Tags, Tax, Emps> {
    type Output = Result<Policies<Cats, Fields, Tags, Tax, Emps>, Error>;
}
```

Users almost never write these type names — inference through the fluent
chain carries them. When a `Policy` must cross a function boundary, the
caller names the states they rely on (e.g.
`Policy<Fetched, Omitted, Omitted, Fetched, Omitted>`); that verbosity is
the accepted cost, noted here as a judgement call.

### The dynamic escape hatch, and what it costs

```rust
pub enum PolicyField { Categories, ReportFields, Tags, Tax, Employees }  // closed; Copy, Eq, Hash

impl Client {
    pub fn get_policies_dynamic(&self, ids: impl IntoIterator<Item: Into<PolicyId>>,
                                fields: impl IntoIterator<Item: Into<PolicyField>>)
        -> GetPoliciesDynamicAction;                       // #[must_use]; on_behalf_of(email)
}
impl IntoFuture for GetPoliciesDynamicAction { type Output = Result<DynamicPolicies, Error>; }

pub struct DynamicPolicy { categories: Option<Vec<Category>>, report_fields: ..,
                           tags: Option<PolicyTags>, tax: Option<Option<TaxConfig>>,
                           employees: .. }                 // Clone, Debug; pub fields
pub type DynamicPolicies = HashMap<PolicyId, DynamicPolicy>;

impl<..all FetchState..> Policy<..> { pub fn project(self) -> DynamicPolicy }
```

This is a real concession, and worth naming as one: everything above argues
that a request-dependent `Option` is the defect this mechanism exists to
delete, and `DynamicPolicy` puts five of them back. `tax` gets the shape the
design explicitly set out to avoid — `Option<Option<TaxConfig>>`, outer
request-dependent, inner data-dependent — where `Policy` keeps those two
kinds of absence in different type positions.

It earns its place because the alternative is worse for the caller it serves.
A caller whose selection is *data* (argv flags, a config file, an RPC field)
cannot reach `GetPoliciesAction` at all without branching over the states,
and the CLI's attempt is the evidence: a five-function generic ladder,
one stage per type parameter, whose only purpose was to turn five booleans
into 32 monomorphized leaves. That code is not safer than an `Option` — it is
an `Option` written in type parameters, plus ~90 lines that must stay in sync
with the field list. Reading the flags as data is the honest spelling of a
runtime fact.

What is given up, precisely:

- Reading an unrequested section is `None` instead of a compile error
  (misuse 4 does not apply to `DynamicPolicy`).
- "At least one field" degrades from a type-level proof (`GetPoliciesBuilder`
  is not a future — misuse 5) to `Error::InvalidRequest` at `.await`. Same
  class as case 7's empty-iterator residue: the API rejects it, no type can
  see it, so it is runtime-but-loud and nothing is sent.
- Double-selecting a field is a method-not-found error on the static path
  (misuse 11); here it is deduplicated silently, because a `Vec` can hold
  the same value twice and rejecting it would fail requests that mean
  exactly what they say.

What is *not* given up: the request path. The wire `fields` list was already
runtime state inside `GetPoliciesAction` — only response shaping was ever
static — so both getters call one private `fetch`, which validates, sends and
splits the response into undecoded sections. The dynamic getter then decodes
each requested section through `Fetched::extract`, the same hook the static
one uses, so a section that was requested and did not come back is the same
`DecodeError` on both. There is one request path and one wire-name table
(`PolicyField::wire`), which the `with_*` setters push into as well.

`get_policies` stays the documented default; `get_policies_dynamic`'s rustdoc
says outright that it reintroduces the `unwrap` and names the case it is for.

## Reimbursement: strict vs tolerant partial success

The silent-partial bug — code assumes everything was reimbursed and never
looks at `skippedReports` — is closed with the `verbose_results` move applied
to error strictness:

```rust
impl Client { pub fn mark_reports_reimbursed(&self, targets: ReimburseTargets) -> ReimburseAction<Strict> }

pub struct Strict; pub struct Tolerant;                 // no public trait needed; two impls only
pub struct ReimburseAction<Mode = Strict>;              // .payment_source(..) on all modes
impl ReimburseAction<Strict> { pub fn tolerate_partial(self) -> ReimburseAction<Tolerant> }

impl IntoFuture for ReimburseAction<Strict>   { type Output = Result<Vec<ReportId>, Error>; }
    // non-empty skipped/failed (or 207) -> Err(Error::PartialSuccess(Box<..>));
    // Ok has no lists to forget to check
impl IntoFuture for ReimburseAction<Tolerant> { type Output = Result<ReimburseOutcome, Error>; }
    // whatever the code, the outcome is Ok
```

**Strictness is keyed on the lists, not on `responseCode`, and that is not a
belt-and-braces choice — it is the only thing that works.** Both observed
partial runs came back **200**: three Open reports with everything skipped and
`reportIDs: []`, and a mixed batch of one Approved plus two Open with
`reportIDs: ["R00X9oNOn2MO"]` and two `skippedReports`. Expensify has not been
seen answering 207 at all. Keying on 207 — which the docs imply, and which
this crate did — made the first case `Ok(vec![])` and the second
`Ok(["R00X9oNOn2MO"])`: success, with every skip reason discarded, and in the
mixed case indistinguishable from a one-report run that worked. The 207 branch
is kept because the docs describe it and it costs nothing, but it is dead code
against every response seen so far.

`failedReports` is **absent** from both bodies rather than `[]`, so it
deserializes with `#[serde(default)]`. Hand-written mocks tend to include the
key; the replayed fixtures do not, which is how that stays honest.

```rust
pub struct ReimburseTargets;   // anchored: report_ids(..) | since(Date); .until(Date)
pub struct ReimburseOutcome { pub updated: Vec<ReportId>,
                              pub skipped: Vec<SkippedReport>,   // wrong status
                              pub failed:  Vec<SkippedReport> }  // other reasons
pub struct SkippedReport { pub report_id: ReportId, pub reason: String }
```

There is no status parameter anywhere: `REIMBURSED` is the only value
Expensify accepts, so the method name *is* the status.

## Operation catalog

| Expensify job | method | action | `IntoFuture::Output` (Ok type) |
|---|---|---|---|
| Report Exporter (`file`/`combinedReportData`) | `Client::export_reports(&ExportTemplate<F>, ReportsQuery)` | `ExportReportsAction<F>` | `ExportedFile<F>` |
| Downloader (`download`) | `Client::download(&ExportedFile<F>)` | `DownloadAction<F>` | `F::Output` |
| Reconciliation (`reconciliation`) | `DomainClient::reconcile(&ReconciliationTemplate<F>, start, end, scope)` | `ReconcileAction<F>` | `ExportedFile<F>` |
| Policy List Getter (`get`/`policyList`) | `Client::list_policies()` | `ListPoliciesAction` | `Vec<PolicySummary>` |
| Policy Getter (`get`/`policy`) | `Client::get_policies(ids)` | `GetPoliciesBuilder` → `GetPoliciesAction<..>` | `HashMap<PolicyId, Policy<..>>` |
| Policy Getter, runtime selection | `Client::get_policies_dynamic(ids, fields)` | `GetPoliciesDynamicAction` | `HashMap<PolicyId, DynamicPolicy>` |
| Domain Cards Getter (`get`/`domainCardList`) | `DomainClient::card_list()` | `DomainCardListAction` | `Vec<DomainCard>` |
| Policy Creator (`create`/`policy`) | `Client::create_policy(name)` | `CreatePolicyAction` | `CreatedPolicy` |
| Report Creator (`create`/`report`) | `Client::create_report(policy, email, title, expenses)` | `CreateReportAction` | `CreatedReport` |
| Expense Creator (`create`/`expenses`) | `Client::create_expenses(employee_email, expenses)` | `CreateExpensesAction` | `Vec<CreatedTransaction>` |
| Expense Rules Creator (`create`/`expenseRules`) | `Client::create_expense_rule(policy, email)` | `CreateExpenseRuleAction` | `()` |
| Expense Rules Updater (`update`/`expenseRules`) | `Client::update_expense_rule(policy, email, RuleId)` | `UpdateExpenseRuleAction` | `()` |
| Policy Updater (`update`/`policy`) | `Client::update_policy(id)` / `update_policies(ids)` | `UpdatePolicyAction` | `()` |
| Report Status Updater (`update`/`reportStatus`) | `Client::mark_reports_reimbursed(targets)` | `ReimburseAction<Strict\|Tolerant>` | `Vec<ReportId>` / `ReimburseOutcome` |
| Tag Approvers Updater (`update`/`tagApprovers`) | `Client::set_tag_approvers(policy, approvers)` | `SetTagApproversAction` | `()` |
| Advanced Employee Updater (`update`/`employees`) | `Client::update_employees(EmployeeSource)` | `UpdateEmployeesAction` | `EmployeeUpdateOutcome` |
| Employee Updater, deprecated | `Client::update_employees_csv(policy, csv)` *(feature)* | `UpdateEmployeesCsvAction` | `u64` (nbEmployees) |

## Type inventory

Signatures are spelled fully in `src/` — this section records fields,
derives, and intent per module. Derives shorthand: **SD** =
`Serialize, Deserialize`; ids also get `PartialEq, Eq, Hash`.

### `types.rs` — newtypes and money

- `PolicyId`, `ReportId`, `TransactionId`, `TaxRateId` — string newtypes
  (macro-generated): Clone, Debug, PartialEq, Eq, Hash, SD
  (`#[serde(transparent)]`), `Display`; `new`, `as_str`;
  `From<String> / &str / &Self`. Every API surface takes
  `impl Into<XxxId>` so `"literal"` works but a *different* id type never
  does.
- `RuleId(pub i64)` — Copy + the above (integer on the wire).
- `Currency(String)` — unvalidated ISO-4217-ish code; same derives.
- `Money { pub cents: i64, pub currency: Currency }` — `Money::new(cents,
  currency)`. Pairs the two fields Expensify always requires together;
  bare `i64` cents appear only in parameters explicitly named `*_cents`.

### `secret.rs`

`Secret<T = String>` (Clone, PartialEq, Eq; `Debug`/`Display` redact; no
`Serialize`, no `Deref`), `MaskedUrl` (same, masking only the userinfo). See
[§ Secrets](#secrets).

### `observe.rs`

`Observer` (trait; `on_request` defaults to a no-op, blanket impl for
`Fn(&Exchange)`), `ObservedRequest`, `Exchange`, `Recorder` — all Clone,
Debug, and the two request/response types also Display. See
[§ Observability](#observability).

### `error.rs`

```rust
#[non_exhaustive] pub enum Error {          // thiserror
    Transport(#[from] reqwest::Error),
    RateLimited { retry_after: Option<Duration> },       // HTTP 429 or body code 429
    Api(ApiError),                                       // body responseCode != 200/207, any HTTP status
    InvalidRequest(String),                              // rejected before sending; see misuse 7
    Http { status: reqwest::StatusCode, body: String },  // non-success HTTP, unrecognizable body
    Decode(#[from] DecodeError),                         // envelope decode or FromExport failure
    PartialSuccess(Box<ReimburseOutcome>),               // 207 on the strict reimburse path only
}
pub struct ApiError { pub kind: ApiErrorKind, pub code: u16, pub message: Option<String> }
#[non_exhaustive] pub enum ApiErrorKind { InvalidPermissions /*403*/, NotFound /*404*/,
                                          Validation /*410*/, Server /*500*/, Other }
#[non_exhaustive] pub enum DecodeError { Json(#[from] serde_json::Error),
                                         Utf8(#[from] FromUtf8Error), Custom(String) }
impl DecodeError { pub fn custom(impl Into<String>) -> Self }   // for user FromExport impls
```

HTTP 200 never implies success: the wire layer parses the body envelope
first and maps its `responseCode`. 429 from either layer becomes
`RateLimited` (`retry_after` from the header if present — likely absent,
hence `Option`). `InvalidRequest` is the pre-flight rejection for requests
the type system cannot refuse but Expensify documents as a 410 — today,
only empty collections. `PartialSuccess` is the one operation-specific variant;
accepted because 207 is a documented cross-cutting code that today only
the reimburse job emits, and a per-op error enum for one job is worse.

### `template.rs` / `file.rs`

Covered in [§ Exports](#exports-templates-files-download). Inventory:
`FromExport` (open trait), `Json<T>` (marker), `ExportTemplate<F = Bytes>`,
`ReconciliationTemplate<F = Bytes>` (manual Clone/Debug),
`FileSystem` (Clone, Copy, Debug, PartialEq, Eq, Hash, SD),
`ExportedFile<F = Bytes>` (manual Clone/Debug; SD with
`bound = ""`, phantom `#[serde(skip)]`), `DownloadAction<F>`.

### `export.rs`

`ReportsQuery` (Clone, Debug), `ReportState` (Copy, Eq — Open, Submitted,
Approved, Reimbursed, Archived), `ExportFormat` (Copy, Eq — Csv, Xls,
Xlsx, Txt, Json, Xml; **no `Pdf`** — see open question 4),
`SftpConnection` (Clone, Debug, pub fields; `password: Secret<String>`),
`OnFinish` (Clone, Debug;
private enum inside), `EmailOnFinish` (Clone, Debug; `Into<OnFinish>`),
`ExportReportsAction<F>`.

### `reconciliation.rs`

`ReconciliationScope` (Copy, Eq — Unreported, All), `ReconciliationFormat`
(Copy, Eq — Csv, Txt, Json, Xml), `ReconcileAction<F>`.

### `policy/` (flattened via `pub use` in `mod.rs`)

- `model.rs` — data types shared read/write:
  - `Category` — Clone, Debug, PartialEq, SD. Fields: `name: String`,
    `enabled: bool`, `gl_code`, `payroll_code`, `comment_hint:
    Option<String>`, `are_comments_required: Option<bool>`,
    `max_expense_amount_cents: Option<i64>` (wire `maxExpenseAmount`).
    Builder: `new(name)` (enabled), `.disabled()`, `.gl_code()`,
    `.payroll_code()`, `.require_comments()`, `.comment_hint()`,
    `.max_expense_amount_cents()`.
  - `PolicyTag { name, enabled, gl_code: Option<String> }` — same style;
    `enabled` also defaults to true.
  - `PolicyTagLevel { name: Option<String>, tags: Vec<PolicyTag> }` —
    getter's level shape, Deserialize only. No "required" flag: the
    getter sample does not carry one and inventing a wire key would be a
    guess.
  - `PolicyTags` — Clone, Debug, PartialEq, `#[non_exhaustive]`;
    `Flat(Vec<PolicyTag>)` | `Levels(Vec<PolicyTagLevel>)`, plus
    `.tags()`. Hand-written `Deserialize` (discriminates on the presence
    of a `tags` key), not `#[serde(untagged)]`, whose "did not match any
    variant" swallows the real shape error.
  - `ReportFieldType` — Clone, Eq, Deserialize lowercase,
    `#[non_exhaustive]`: Formula (read-only — the getter emits it), Text,
    Dropdown, Date, `Other(String)`. Read side only.
  - `ReportFieldDefType` — Copy, Eq, Serialize lowercase: Text, Dropdown,
    Date. The updater's documented set. Separate from `ReportFieldType`
    so `Formula` is not statable on a `ReportFieldDef` (misuse 13) and so
    the read side can stay open without opening the write side.
  - `ReportField { name, field_type: ReportFieldType, values: Vec<String> }`
    — getter shape, Deserialize only.
  - `ReportFieldDef { name, field_type: ReportFieldDefType,
    values: Vec<ReportFieldValue>, default_value: Option<String> }` —
    updater shape, Serialize only.
    `new(name, type)`, `.values(iter Into<ReportFieldValue>)`,
    `.default_value()`. Values always serialize in object form, which
    satisfies Expensify's "uniformly strings or uniformly objects" rule
    by construction.
  - `ReportFieldValue { name, enabled, external_id }` —
    `new`, `.disabled()`, `.external_id()`, `From<&str>/<String>`.
  - `TaxConfig { name, default: TaxRateId, rates: Vec<TaxRate> }`;
    `TaxRate { name, rate: f64, rate_id: TaxRateId }` — Deserialize.
  - `PolicyRole` — Clone, Eq, SD lowercase, `#[non_exhaustive]`: User,
    Auditor, Admin, `Other(String)`. Not `Copy`: the catch-all carries the
    raw string. It is also the one open enum this crate *sends*
    (`Employee::role`); an `Other` there is a server-side rejection, which
    is the accepted cost of one type serving both directions.
  - `PolicyEmployee { email, role, submits_to, employee_id,
    custom_field_1, custom_field_2 }` — Deserialize.
  - `PolicyPlan` — Clone, Eq, SD lowercase, `#[non_exhaustive]`: Team,
    Corporate, `Other(String)` (wire key `type`; renamed to avoid a third
    meaning of "type"). `free`, `control` and `personalPolicy` are all
    observed and undocumented.
  - `PolicySummary { id, name, owner, role, output_currency, plan }` —
    Deserialize.
- `get.rs` — `Payload`, `FetchState` (sealed; `Wrap` GAT + `project`),
  `Fetched`, `Omitted`, `NotFetched`, `Policy<..>` (+ `.project()`),
  `Policies<..>` alias, `GetPoliciesBuilder`, `GetPoliciesAction<..>`, and
  the runtime-selection half: `PolicyField` (Copy, Eq, Hash; closed),
  `DynamicPolicy` (Clone, Debug; pub `Option` fields), `DynamicPolicies`
  alias, `GetPoliciesDynamicAction`. See
  [§ Policy getter](#policy-getter-fetch-flags-as-typestate).
- `list.rs` — `ListPoliciesAction`: `.admin_only()`, `.on_behalf_of()`.
- `create.rs` — `CreatePolicyAction`: `.plan(PolicyPlan)`;
  `CreatedPolicy { policy_id, name }`.
- `update.rs` — `CategoriesUpdate` / `ReportFieldsUpdate`
  (`merge(iter)` / `replace_all(iter)` — the destructive mode is named
  destructively); `TagLevel { name: Option, required, tags }`
  (`new(tags)`, `.named()`, `.required()`); `TagCsvConfig` — constructors
  `dependent(set_required: bool)` and
  `independent(set_required: impl IntoIterator<Item = bool>)` mirror
  Expensify's rule that `setRequired` is scalar for dependent levels and
  per-level for independent ones (the wrong pairing is unrepresentable);
  `.with_gl_codes()`, `.with_header_row()`, `.tsv()`; `TagsUpdate` —
  `replace_all_inline` (independent levels only — the inline form simply
  has no dependency knob, matching the API) and `replace_all_csv(data,
  config)` (the CSV goes in the `file` form field), **replace-only —
  permanently**, now that `action: "merge"` is confirmed destructive
  (open question 9); `UpdatePolicyAction` — `.categories()`,
  `.report_fields()`, `.tags()`, each optional and independent.
- `approvers.rs` — `TagApprover::assign(tag, email)` /
  `TagApprover::clear(tag)` (clearing is an explicit constructor, not the
  empty-string wire sentinel); `SetTagApproversAction` (no setters).

### `expenses.rs`

- `ExpenseTax` — `new(rate_id)`, `.amount_cents(i64)`; rate IDs come from
  `get_policies(..).with_tax()`.
- `Expense` — `new(merchant, date: time::Date, amount: Money)`; setters
  `external_id`, `category`, `tag`, `billable(bool)`,
  `reimbursable(bool)`, `comment`, `report_id(impl Into<ReportId>)`,
  `policy_id(..)`, `tax(ExpenseTax)`. Clone, Debug; fields private.
- `CreateExpensesAction` — no setters. `employee_email` is a **required
  argument** of `create_expenses`: Expensify answers 410 (`'employeeEmail'
  parameter is missing or malformed`) without it, with or without a policy on
  the expenses, and does not fall back to the credential owner as the docs
  say. The same docs call the parameter restricted and needing advanced
  permissions; a plain policy-admin trial account used it with no grant, so
  that claim is suspect too.
- `CreatedTransaction { transaction_id, report_id: Option<ReportId>, merchant,
  created: Date, amount_cents, currency }` — Clone, Debug. `report_id` records
  an **undocumented side effect**: an expense created without
  `Expense::report_id` is not left loose — Expensify opens a report for it and
  names that report in the response. Discarding it left callers unable to find
  their own expense without a separate export.

  `Option`, though it has been present on every observed response, and this is
  the one place in this crate where a request-independent field is optional on
  purpose rather than because absence is data. The rule elsewhere — a value
  that should be there and is not is a `DecodeError`, never a silent `None` —
  assumes a decode failure means the caller got nothing. Here it means the
  opposite: **the expense already exists** by the time this decodes. Requiring
  the field would turn a created expense into an error that the caller can
  neither act on nor safely retry (retrying duplicates it), over a field
  describing a side effect rather than the transaction. `None` costs them the
  report's name and nothing else. `tests/replay.rs` covers the branch no
  recorded body can.

  The response also carries `comment`, `tag`, `category` and `mcc`, which only
  echo the request (or Expensify's default for it, e.g. `"Uncategorized"`) and
  are not modelled; no wire struct sets `deny_unknown_fields`, so a response
  that grows a field does not fail a created expense.

### `reports.rs`

- `ExpenseLine` — `new(merchant, date, amount: Money)` only. Deliberately
  narrower than `Expense`: the report-creator job accepts exactly these
  four wire fields, so category/tag/etc. cannot be attached and silently
  dropped.
- `CreateReportAction` — `.report_field(name, value)` (repeatable),
  `.report_fields(&impl Serialize)` (must serialize to a JSON object;
  serialized eagerly, failure surfaces at `.await`). Keys normalized
  (non-alphanumeric → `_`) before sending. The docs say the job requires
  support-side enablement + domain/policy admin; it worked immediately on a
  trial account with policy-admin rights and no unlock, so that requirement is
  **unconfirmed** rather than disproved — one account is not a general rule.
  `CreatedReport { report_id, name }`.
- `ReimburseTargets`, `Strict`, `Tolerant`, `ReimburseAction<Mode>`,
  `ReimburseOutcome`, `SkippedReport` — see
  [§ Reimbursement](#reimbursement-strict-vs-tolerant-partial-success).

### `expense_rules.rs`

`CreateExpenseRuleAction` / `UpdateExpenseRuleAction` — `.tag(..)`,
`.default_billable(bool)`; at least one is required (runtime 410 — two
optional knobs don't justify typestate). Output `()` because Expensify
documents no response body for these jobs.

### `employees.rs`

- `Employee` — `new(employee_email, manager_email, employee_id,
  policy_id)` + setters for every documented optional (`first_name`,
  `last_name`, `custom_field_1/2`, `approval_limit(i64)`,
  `over_limit_approver`, `worker_status`, `terminated()`,
  `domain_group_id`, `approves_to`, `role(PolicyRole)`,
  `additional_policy_ids`, `remove_from_unassigned_policies()`,
  `default_tags`). Doc notes carried onto methods: `employee_id` drives
  email-change/merge detection and auto-fills Custom Field 1;
  `domain_group_id` only applies if every record has one.
- `EmployeeSource` — `Inline(Vec<Employee>)` (`dataSource: "request"`),
  `FetchUrl { url: MaskedUrl, user, password: Option<Secret<String>> }`
  (`"download"`), `Sftp { connection: SftpConnection, filename }`
  (`"sftp"`). An enum, not typestate: three mutually exclusive wire shapes,
  no sequencing. Clone, Debug — both derived, because the field types
  redact. `https://user:pass@host/feed.json` is a natural way to spell a
  basic-auth feed, so the URL is a secret carrier too; `MaskedUrl` keeps its
  host and path, which is why it is printed at all. Same treatment for a
  caller-set `base_url` (`url::Url`'s own `Debug` prints `password`
  verbatim). See § Secrets.
- `PrimaryPolicyMode` — None, NewEmployees, AllEmployees.
- `UpdateEmployeesAction` — `.dry_run()`, `.primary_policy(mode)`,
  `.no_approval_chain_fixes()` (server default on),
  `.first_level_managers_only()`, `.skip_notification_emails()`,
  `.email_on_finish(recipients)`.
- `EmployeeUpdateOutcome { dry_run, updated_count,
  added: HashMap<PolicyId, Vec<String>>, removed: ..,
  security_group_assignments: HashMap<String, Vec<String>>,
  skipped: Vec<SkippedEmployee> }`; `SkippedEmployee { email, reason }`.
- Feature-gated + `#[deprecated]`: `UpdateEmployeesCsvAction`
  (multipart; Output `u64`).

### `cards.rs`

`DomainCard { bank, card_id: i64, card_name, card_number /*masked*/,
email, external_employee_id: Option, created: Option<PrimitiveDateTime>,
last_import: Option<PrimitiveDateTime>, last_import_result: Option<u16>,
reimbursable: bool, scrape_min_date: Option<Date> /* "" → None */ }`;
`DomainCardListAction` (no options).

### Private modules (implementer-owned, no public surface)

- `wire.rs` — envelope assembly (`requestJobDescription` + extra form
  fields), response envelope parsing (body `responseCode` precedence,
  downloader raw-body special case), all serde renames that don't map
  1:1 from public types.
- `limit.rs` / `RateGate` in `client.rs` — governor plumbing.

## Rate limiting

Built-in and on by default; invisible per-operation. `RateGate` holds two
`governor::DefaultDirectRateLimiter`s (5/10 s and 20/60 s); every
send awaits both.

The quota spelling matters and is easy to get wrong: GCRA admits
`burst + elapsed/period` cells, so `with_period(window / budget)
.allow_burst(budget)` — the obvious reading — admits the burst *on top of*
a full window's replenishment, roughly double the published rate on a cold
start or after any idle gap. The limiter exists only to prevent 429s, so it
keeps the implicit burst of one and spreads the remaining `budget - 1`
cells across the window, making `budget` per `window` an upper bound at
every offset. Only the first send is instant; the 60 s budget is the
binding one thereafter (~3.2 s between sends). Deliberately
under-shooting: a client that is slightly too slow is strictly better than
one that gets 429ed.

Opt-out is `ClientBuilder::no_rate_limiting()` — the
limiter is process-local, so multi-process deployments sharing one
credential need an external governor and will still see
`Error::RateLimited`, which is why 429 remains a surfaced error rather
than an internal retry. No auto-retry in v1 (see Open questions).

## Dependencies & features

| crate | why |
|---|---|
| `reqwest` (no default features; `rustls-tls`, `charset`, `http2`) | HTTP; `multipart` pulled in only by the deprecated-updater feature. Re-exported (`expensify::reqwest`), so its major version is part of this crate's semver surface — it already was, via four types in public signatures |
| `serde` (+derive), `serde_json` | envelope + user-type bounds (`Serialize`/`DeserializeOwned`) |
| `bytes` | zero-copy download payloads; `ExportedFile<Bytes>` default |
| `time` (`serde`, `macros`, `formatting`, `parsing`) | `Date`/`PrimitiveDateTime` in the public API; lighter than chrono |
| `governor` | the two-window rate limiter |
| `thiserror` | error derives |
| dev: `tokio` (macros, rt) | examples/tests only — the lib itself is runtime-agnostic-by-accident (reqwest requires tokio anyway, so no abstraction layer is designed) |

Unchanged by observability: the callback hook needs nothing `std` does not
already provide, which is half of why it was chosen over `tracing`. See
[§ Observability](#observability). The CLI (a separate crate, not on this
table) turned on `tracing-subscriber`'s `registry` feature so it can filter
by target — a single global level cannot show our diagnostics without also
showing `h2`'s frame handling.

Features: `employee-updater-deprecated` (default off) gates the one
multipart job and its reqwest feature — deprecated upstream, and the only
consumer of multipart. No other flags: everything else is small, and
flag combinatorics cost more than they save here.

## Module layout

```
src/
  lib.rs            # glob re-exports (flat public namespace), BoxFuture alias
  client.rs         # Credentials, Client, ClientBuilder, DomainClient, RateGate
  error.rs
  types.rs          # id newtypes, Currency, Money
  template.rs       # FromExport, Json, ExportTemplate, ReconciliationTemplate
  file.rs           # FileSystem, ExportedFile, DownloadAction
  export.rs         # ReportsQuery, ReportState, ExportFormat, OnFinish, EmailOnFinish, SftpConnection, ExportReportsAction
  reconciliation.rs # ReconciliationScope, ReconciliationFormat, ReconcileAction
  observe.rs        # Observer, ObservedRequest, Exchange, Recorder
  secret.rs         # Secret<T>, MaskedUrl
  policy/           # mod, model, get, list, create, update, approvers
  expenses.rs       # Expense, ExpenseTax, CreateExpensesAction, CreatedTransaction
  reports.rs        # ExpenseLine, CreateReportAction, ReimburseTargets/Action/Outcome
  expense_rules.rs
  employees.rs      # advanced updater + deprecated (feature)
  cards.rs          # DomainCard, DomainCardListAction
  wire.rs, limit.rs # private: envelope assembly/parsing, rate-limit plumbing
```

## Misuses made uncompilable

Each entry is a `trybuild` case under `tests/ui/` with a committed
`.stderr`; error classes below are quoted from rustc.

1. **Wrong `fileSystem` for a filename.** The classic: reconciliation
   filename + default `integrationServer` download = 404/garbage.
   ```rust
   ExportedFile::<Bytes> { name: "is_reconciliation_123.csv".into(),
                           file_system: FileSystem::IntegrationServer }
   // error: cannot construct `ExportedFile` with struct literal syntax due to private fields
   ```
   There is no other spelling: `download()` has no file-system parameter.
2. **Decoding an export as the wrong type.**
   ```rust
   let file /* : ExportedFile<Json<Vec<Row>>> */ = client.export_reports(&template, q).await?;
   let rows: Vec<String> = client.download(&file).await?;   // E0308 mismatched types
   ```
3. **Export template fed to reconciliation** (disjoint FreeMarker data
   models): `dc.reconcile(&export_template, ..)` — E0308, expected
   `&ReconciliationTemplate<_>`.
4. **Reading a policy field that wasn't requested** — the 5× unwrap
   killer:
   ```rust
   let p = client.get_policies([id]).with_tax().await?;
   for c in &p[&id].categories {}   // E0277: `&NotFetched` is not an iterator
   ```
5. **Awaiting the policy getter with no fields** (documented 410):
   `client.get_policies([id]).await` — E0277: `GetPoliciesBuilder` is not
   a future.
   Scope: this is the *static* getter's guarantee.
   `get_policies_dynamic(ids, [])` is well-typed for the same reason an
   empty iterator is (case 7), and is rejected at `.await` with
   `Error::InvalidRequest`.
6. **ID swaps**: `.report_id(policy_id)` — E0277:
   `ReportId: From<PolicyId>` not satisfied.
7. **A filterless export constructor** (documented 410): `ReportsQuery`
   has only anchored constructors — `ReportsQuery::default()` is E0599.
   **Scope, honestly:** this closes the *spelling*, not the request. An
   empty iterator is well-typed — `ReportsQuery::report_ids([])`,
   `ReimburseTargets::report_ids([])`, `get_policies([])`,
   `update_policies([])` all anchor nothing and would serialize the exact
   `"filters": {}` / empty `policyIDList` the API answers 410 to. No type
   can see the length of an iterator, so those four are **runtime**-
   rejected instead: awaiting returns `Error::InvalidRequest` and nothing
   is sent.
8. **Silently ignoring partial reimbursement**: strict output is
   `Vec<ReportId>` — `outcome.skipped` is E0609; there is no list to
   forget. Reaching `ReimburseOutcome` requires typing `.tolerate_partial()`.
   **The type half was never the leaky half.** The runtime half — deciding
   *when* to withhold that `Ok` — keyed on `responseCode == 207`, and
   Expensify answers partial runs with 200, so the strict path returned a
   short `Ok(Vec<ReportId>)` and discarded the reasons. Fixed by keying on the
   skipped/failed lists; see
   [§ Reimbursement](#reimbursement-strict-vs-tolerant-partial-success). The
   compile error quoted here is unchanged, which is exactly why it never
   caught this.
9. **Any report-status value other than reimbursed**: no status
   parameter exists to hold `"APPROVED"` (E0599 on any invented method).
10. **Rich expense fields on the report creator** (would be silently
    dropped by the four-field wire shape):
    `ExpenseLine::new(..).category("Meals")` — E0599.
11. **Double-selecting a policy field**:
    `.with_tax().with_tax()` — E0599 (`with_tax` exists only while that
    slot is `Omitted`). The dynamic getter cannot borrow this —
    a `Vec<PolicyField>` can hold a repeat — so it deduplicates instead.
12. **`setRequired` shape mismatch on tag CSVs** (scalar vs per-level):
    unrepresentable — `TagCsvConfig::dependent` takes `bool`,
    `::independent` takes an iterator; there is no field to set wrongly.
13. **A formula report field on the updater** (the updater documents only
    text/dropdown/date): `ReportFieldDef::new(name,
    ReportFieldType::Formula)` — E0308, the constructor takes the narrower
    `ReportFieldDefType`.
14. **A typed decode conjured from a bare filename**:
    `ExportedFile::<Json<Row>>::from_parts(..)` — E0599; the escape hatch
    exists only on `ExportedFile<Bytes>`. (Not airtight — see the serde
    caveat in [§ Exports](#exports-templates-files-download).)
15. **A third `FetchState`**: `impl FetchState for Mine` — E0277, the
    supertrait is private. A third state has no meaning and would break
    `extract`'s contract.
16. **Reconciliation template fed to the exporter** — the reverse of case
    3; E0308, expected `&ExportTemplate<_>`.
17. **An `onFinish` message on an action that has none.** `message` is
    carried only by the email action, so on any other one it would compile
    and be dropped: `OnFinish::mark_as_exported("x").message("y")` — E0599.
    `OnFinish::email` returns `EmailOnFinish`, which is where `message`
    lives; `on_finish` takes `impl Into<OnFinish>`.
18. **An exporter-only format on the reconciliation job** (which accepts
    only csv/txt/json/xml): `.format(ExportFormat::Xlsx)` — E0308, the
    setter takes the narrower `ReconciliationFormat`. Same split as case 13.

19. **Reading a secret without saying so.** `Secret` is not a smart pointer:
    `&*secret` is E0614, "type `Secret` cannot be dereferenced". `expose()`
    is the only read path, which is what makes every use of a secret
    greppable — and what lets the wire layer be the only place that calls it.
20. **Merging policy tags.** `TagsUpdate::merge_inline(..)` — E0599; the
    constructors are `replace_all_*` only. Withheld on suspicion in 0.1.0,
    kept on evidence: `action: "merge"` deleted every unlisted tag and
    answered `{"responseCode":200}`.

Runtime-but-loud (not compile errors, by design): empty collections are
`Error::InvalidRequest` before anything is sent (case 7); destructive tag/
category replacement is spelled `replace_all_*`; clearing a tag approver
is `TagApprover::clear`, not an empty string; every action is
`#[must_use]`.

Two operations are **withheld** rather than made uncompilable, because the
misuse is the server's behaviour and not a spelling: `TagsUpdate`'s
`merge_*` constructors (open question 9) and `ExportFormat::Pdf` (open
question 4). The first is now settled — merge is destructive, so it stays
absent for good, and case 20 pins that. The second still ships absent pending
a probe; adding it back is additive, not a breaking change.

## Naming divergences from the wire

`snake_case` throughout; beyond mechanical case mapping:

| wire | here | why |
|---|---|---|
| `type` (policy plan) | `PolicyPlan::{Team, Corporate, Other}` | `type` already means job and inputSettings discriminators |
| `type` (report field) | `ReportFieldType` (read) / `ReportFieldDefType` (write) | one wire key, two vocabularies — the updater rejects `formula` |
| `filters.markedAsExported` | `not_yet_exported_as(label)` | the filter *excludes* already-exported reports; wire name reads inverted |
| `maxExpenseAmount`, `amount` | `*_cents` / `Money` | cents-vs-currency-units is the classic money bug |
| `reportStatus` update | `mark_reports_reimbursed` | only one status exists; encode it in the verb |
| `dataSource: request/download/sftp` | `EmployeeSource::{Inline, FetchUrl, Sftp}` | "download" collides with the Downloader job; "request" is meaningless out of context |
| `dry-run` (hyphenated key) | `.dry_run()` | wire quirk stays in wire.rs |
| `fileType: "cvs"` | `TagCsvConfig` (+ `.tsv()`) | upstream typo; send `"csv"` |

## Rejected mechanisms

- **Capability typestate** (domain-admin client, report-creation-enabled
  client). The library cannot verify either fact; a
  `client.assume_domain_admin()` phantom would encode an unverified user
  assertion, and the failure mode (403/500 at runtime) would be
  unchanged — ceremony without prevention. `DomainClient` survives only
  because the domain *string* is required job data. Permission
  requirements are doc comments plus `ApiErrorKind::InvalidPermissions`.
- **Typestate for "≥1 filter" on `ReportsQuery`** — anchored
  constructors get most of the way with zero type parameters and better
  error messages. They cannot see an empty iterator, and no type
  parameterization can, so that residue is a runtime check
  (`Error::InvalidRequest`) rather than a reason to add five type
  parameters that would still not close it.
- **`Option<TaxConfig>` elimination** — kept: `{}` on the wire is
  genuine data-dependent absence, exactly what `Option` is for.
- **Generic `WriteModel`-style job enum** — Expensify jobs share nothing
  but the envelope; a unifying enum would be `serde_json::Value` with
  extra steps. Continuity comes from the action pattern instead.
- **Runtime-agnostic abstraction** — reqwest pins tokio; a runtime trait
  would be speculative complexity with one impl.
- **`FromExport` sealing** — left open deliberately: user-side CSV/XML
  markers are the escape hatch's escape hatch.
- **A third `FetchState` for the dynamic getter** (`Dynamic`, with
  `Wrap<T> = Option<T>`, making `DynamicPolicy` just `Policy<Dynamic, ..>`).
  Tempting — it reuses `Policy` and the existing `IntoFuture` outright — and
  rejected: `extract` would have no way to tell "not requested" from "the
  server left out a section you asked for", so it would have to answer `None`
  to both, turning a decode error into silent missing data. It would also
  make misuse 15's seal an arbitrary line rather than a meaningful one. A
  separate `DynamicPolicy` costs one struct and keeps `extract`'s contract.
- **`ClientBuilder::base_url(impl TryInto<Url>)`** — the ergonomic fix for
  "the URL type is not nameable" was to accept a string. Rejected in favour
  of re-exporting `reqwest`: `build()` cannot fail, so a parse error would
  have to be stashed and surfaced at the first `.await` — converting a
  failure the caller can see at the point of the mistake into one that
  appears later, from a different call, which is the trade this crate makes
  in the other direction everywhere else. The re-export also fixes the three
  *other* reqwest types in the public API, which `TryInto` would not touch.
  `Url::parse` at the call site stays one line.

## Verification status

Confidence in this wire layer is **not uniform**, and reading it as if it were
is what let five defects ship at once. Every response shape below carries how
it is known:

- **observed** — a real body from a live account, recorded through
  `ClientBuilder::observe` and replayed in `tests/replay.rs`;
- **doc example** — Expensify's documentation shows a worked example of *this
  job's* response;
- **inferred** — no job-specific example; the shape is generalized from
  another job or from a parameter table.

| Response shape | Status | Notes |
|---|---|---|
| Policy List Getter → `policyList` | observed | correct as modelled; undocumented plans (`free`, `control`, `personalPolicy`) is why `PolicyPlan` is open |
| Policy Getter, all five sections | observed | correct, both tag shapes included |
| Downloader → raw file body | observed | correct |
| Policy Creator → `policyID`/`policyName` | observed | correct; answered as `application/json` |
| Expense Creator → `transactionList` | observed | envelope and fields as modelled, plus `reportID` (now surfaced — see below) and four echo fields (`comment`, `tag`, `mcc`, `category`) that are ignored |
| Expense Creator auto-creates a report | observed | **undocumented.** An expense that named no report comes back with a `reportID` Expensify opened for it. Surfaced as `Option` anyway — one observation, and a decode failure here would report a created expense as an error |
| Policy Updater (tags, replace) | observed | correct |
| Expense Rules Creator → `{"responseMessage":"OK","responseCode":200}` | observed | `()` is right — no rule ID exists to return |
| **Report Exporter submit → bare filename** | observed | **was wrong**: parsed as an envelope, so the flagship operation never worked |
| **Reimburse, all skipped → 200** | observed | **was wrong**: 207-only strictness reported it as success |
| **Reimburse, mixed → 200** | observed | same; `Ok` was short and looked complete |
| **Expense Creator without `employeeEmail` → 410** | observed | **was wrong**: documented as optional, defaulting to the credential owner |
| **Policy Updater tags, `action: "merge"` → 200** | observed | **was wrong** in the docs, not here: merge deletes unlisted tags. Withheld already, now on evidence |
| Report Status Updater, non-`REIMBURSED` → 410 | observed | `Status '…' is not supported` for SUBMITTED/APPROVED/CLOSED — confirms misuse 9 |
| Undocumented `responseCode: 666` | observed | `Rule already exists with those actions, please update rule N`; maps to `ApiErrorKind::Other` |
| Reconciliation submit → `filename` envelope | doc example | **needs a domain-admin credential; unconfirmed.** Now known to differ from the exporter in at least this respect, so it is no longer safe to assume they match |
| Report Creator → `reportID`/`reportName` | doc example | the response; the *permission* claim around it is separately unconfirmed (see below) |
| Domain Cards Getter → `domainCardList` | doc example | blank-vs-null handling is inferred from the field descriptions |
| Advanced Employee Updater → diff/skipped | doc example | |
| Deprecated CSV updater → `nbEmployees` | doc example | |
| Policy Updater categories/report fields (merge + replace) | doc example | only the tags path was probed |
| Tag Approvers Updater → no body | doc example | |
| Error envelope: body `responseCode` beats HTTP 200 | observed | repeatedly, across jobs |
| 429 / `Retry-After` handling | inferred | never provoked |
| Exporter `test: "true"` as a string | inferred | open question 7, and the highest-consequence guess left |
| PDF export response | inferred | withheld entirely; open question 4 |
| "Not yet rendered" download response | inferred | open question 1 |
| Report-field key normalization | inferred | open question 12 |

**The pattern is worth stating.** Every wrong claim came from inferring
behaviour the docs did not demonstrate *for that specific job* — the exporter
borrowed reconciliation's envelope, the expense creator borrowed a
"defaults to the caller" convention, strictness borrowed 207 from a
cross-cutting code table. Every shape that had a job-specific worked example
was correct. Prose about a job is not evidence about that job; a worked
example is. Two permission claims are also unconfirmed rather than wrong:
`create report` needing a support unlock, and `employeeEmail` needing advanced
permissions — both worked on a policy-admin trial account with neither. One
account cannot prove a requirement never applies, so both stay documented as
unconfirmed.

## Open questions

1. **Downloading a not-yet-rendered export.** Report exports are async
   server-side and the docs don't document the "not ready" response.
   v1 surfaces whatever error comes back; if probing shows a stable
   signal, add `DownloadAction::poll_until_ready(interval, timeout)`.
   Needs a live credential to characterize.

   **Decided in the meantime:** an empty body under HTTP 200 is an
   `Error`, not `Ok(Bytes::new())`. A zero-byte export is never a useful
   result, it is the likeliest shape of "not rendered yet", and reporting
   it as success is the silent-failure class this crate exists to prevent
   — for `Bytes`/`String` markers it would hand an ETL an empty file and
   call the night's run a success. The cost is that a genuinely empty
   export has no accepting path; if probing ever shows one, this is the
   decision to revisit.
2. **429 auto-retry.** Currently surfaced as `Error::RateLimited` with
   no retry. A `ClientBuilder::retry_rate_limited(max: u32)` knob is a
   natural v1.1 addition — rule on whether it belongs in v1.
3. ~~**Expense-rule responses.**~~ **Answered.** The creator returns
   `{"responseMessage":"OK","responseCode":200}` and nothing else, so `()` is
   the right output and there is no `ruleID` to hand back. The only observed
   way to learn a rule's integer ID is an accident: re-creating an identical
   rule answers the **undocumented `responseCode: 666`**, `Rule already exists
   with those actions, please update rule N`. That is a `ApiErrorKind::Other`
   here, with the message intact. Not worth a typed accessor — it would be an
   API built on a collision — but worth knowing before creating rules you
   intend to edit.
4. **PDF exports** (`fileExtension: "pdf"`, one file *per report*): the
   interaction with `returnRandomFileName` (one name vs many) is
   undocumented, and one `ExportedFile` cannot name several files.

   **Decided in the meantime:** `ExportFormat` has no `Pdf` variant. An
   exporter that hands back one handle for forty PDFs is silent partial
   data loss — the caller downloads one file and believes they have all
   forty — which is the failure class this crate exists to prevent, and it
   cannot be prevented by a doc comment on a variant that type-checks.
   `includeFullPageReceiptsPdf` goes with it: the parameter table says it
   "is used only if `fileExtension` contains `pdf`", so with PDF withheld
   the setter could only ever be a no-op. A live probe of the response
   shape decides whether this returns as `Vec<ExportedFile>` or as a
   plain variant.
5. **`ExportFormat` default for `Json<_>` templates** — doc'd default is
   Csv for all; auto-defaulting typed-JSON exports to `json` needs an
   associated const on `FromExport` (small trait wart). Rule on whether
   the ergonomic win justifies it.
6. **Rate-limit figures** — if the "50 jobs/minute" page resurfaces,
   confirm whether it's a separate *job-start* budget on top of the
   request budget; the limiter currently models requests only.
7. **`test` flag encoding — resolved against the earlier guess.** The
   exporter's parameter table types `test` as **String** (`true, false`),
   so the string `"true"` is what goes on the wire; an earlier draft sent
   a JSON boolean. Still unconfirmed live: if the server is
   boolean-typed instead, `.test_run()` is a silent no-op and every
   `onFinish` fires during a believed dry run, including the effectively
   irreversible `markAsExported` and the actually-transmitting `email` /
   `sftpUpload`. Follow the documented type until a live probe says
   otherwise — a wrong `test` is the highest-consequence guess in the
   wire layer, and the docs are the only evidence available.
8. **`reportFields.data.values` object key.** The parameter table says
   `value`; the worked example says `name`. The implementation follows
   the example. The docs contradict themselves; probe.
9. ~~**Whether `action` is honoured for tag updates.**~~ **Answered, and the
   worst way.** Tags were set to Alpha + Beta, then Gamma alone was sent with
   `action: "merge"`. Alpha and Beta were **deleted**; the response was
   `{"responseCode":200}` with no warning. So `action` is read, and "merge"
   means replace. `TagsUpdate` stays replace-only permanently — a `merge_*`
   constructor would be a `replace_all_*` under a name promising the
   opposite — and misuse 20 pins it.
10. ~~**Whether a partial reimbursement is always a 207.**~~ **Answered: it
    is never a 207, so far.** Both probed shapes came back 200 — all-skipped,
    and mixed (one Approved, two Open). The strict path now keys on the
    skipped/failed lists, which is correct under either code; see
    [§ Reimbursement](#reimbursement-strict-vs-tolerant-partial-success).
    What is still unknown is whether 207 ever appears at all; the branch is
    kept for it.
11. **Whether only `REIMBURSED` is accepted — answered, yes.** `SUBMITTED`,
    `APPROVED` and `CLOSED` each return 410, `Status '…' is not supported`.
    Misuse 9 (no status parameter exists) is confirmed rather than assumed.
12. **Report-field key normalization case.** Expensify's rule text
    matches this implementation (non-alphanumeric → `_`, case
    preserved), but its worked example's keys are all lowercase.
    Probably nothing; confirm before adding a `to_lowercase`.
