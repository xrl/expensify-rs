# expensify-rs design

Type-system design for a Rust client for the Expensify Integration Server
API. Deliverable is this document; `src/` holds a compiling skeleton
(`todo!()` bodies) that is signature-authoritative — every public type,
bound, and `IntoFuture` impl below exists there and passes `cargo check`
(both default and `employee-updater-deprecated` feature sets).
`examples/tour.rs` compiles the running example; every entry in
[§ Misuses](#misuses-made-uncompilable) was verified to fail compilation
with the quoted error class.

Source of truth: <https://integrations.expensify.com/Integration-Server/doc/>
and `doc/employeeUpdater/`, read 2026-08-04. No OpenAPI spec exists (the
`openapi.json`/`swagger.json` paths are soft-404s). No versioning or
deprecation signal from Expensify; treat the wire layer as the part most
likely to need maintenance and keep it in one module (`wire.rs`).

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
    .mark_as_exported("acme-etl")
    .await?;                                   // -> ExportedFile<Json<Vec<ReportRow>>>

let rows: Vec<ReportRow> = client.download(&file).await?;   // typed by the file handle

client.create_expenses([
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
**even under HTTP 200** — body code wins. Exception: a successful
Downloader response body is the raw file, not JSON; the implementer must
treat non-200 HTTP or a JSON error envelope as failure and everything
else as file content. All of this lives in `wire.rs` (private); public
types carry serde attrs only where the mapping is 1:1.

Wire mapping notes: all JSON keys camelCase (`#[serde(rename_all)]` or
explicit renames); amounts are integer cents; `reportState` is a
comma-joined list; exporter `limit` serializes as a string; report-field
keys are normalized (non-alphanumeric → `_`) client-side before sending;
`Employee` feed serializes to the documented JSON array;
`TagApprover::clear` serializes `approver: ""`.

## Client, credentials, domain scope

```rust
pub struct Credentials { partner_user_id: String, partner_user_secret: String } // private fields
impl Credentials { pub fn new(id: impl Into<String>, secret: impl Into<String>) -> Self }
// derives: Clone. Debug is MANUAL and redacts the secret — credentials in a
// log line is the bug this prevents. No Serialize.

#[derive(Clone)] pub struct Client { inner: Arc<ClientInner> }   // cheap clone; actions own one
// ClientInner (private): reqwest::Client, Credentials, reqwest::Url, Option<RateGate>

impl Client {
    pub fn new(credentials: Credentials) -> Self;                // prod endpoint, limiter on
    pub fn builder(credentials: Credentials) -> ClientBuilder;
}

pub struct ClientBuilder;   // base_url(reqwest::Url), http_client(reqwest::Client),
                            // no_rate_limiting(), build() -> Client
```

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
#[derive(Clone/*manual*/, Serialize, Deserialize)]      // serde(bound = "") — phantom needs no bounds
pub enum FileSystem { IntegrationServer, Reconciliation }  // renames: integrationServer/reconciliation

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
`ExportedFile<Bytes>` — you can re-assert a file system, but you cannot
conjure a typed decode from a bare string.

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

`ReportsQuery` (anchored constructors; an empty filter set — a documented
410 — is unrepresentable):

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
`.include_full_page_receipts_pdf()`, `.on_finish(OnFinish)` (repeatable),
`.mark_as_exported(label)` (sugar for the common `OnFinish`),
`.test_run()`. Default format is `Csv` for every template marker,
including `Json<_>` — deriving it from `F` would need an associated const
on `FromExport`, which open question 5 rules on. Until then a
`Json<_>` template must call `.format(ExportFormat::Json)`; the mismatch
surfaces as a decode error, not silent corruption.

`OnFinish` constructors: `mark_as_exported(label)`,
`email(recipients).message(text)`, `sftp_upload(SftpConnection)`.
`SftpConnection { host, login, password, port: u16 }` (Clone, Debug —
public fields; shared with the employee updater's SFTP source).

`ReconcileAction<F>` (`DomainClient::reconcile`): required args carry
`start`, `end`, `ReconciliationScope::{Unreported, All}`; setters
`.feed(name)` (default: all feeds), `.format(..)` (Csv/Txt/Json/Xml only,
server-validated), `.email_on_finish(recipients)`. `async` is not
exposed — only `false` is supported upstream, so there is no parameter
(rule 3 of the talk: delete parameters with one useful value).

## Policy getter: fetch-flags as typestate

The Policy Getter's `fields` list decides which response sections exist.
Runtime modeling would be five `Option`s and an `unwrap` at every use
site — the exact shape `verbose_results()` fixed, times five.

```rust
pub trait FetchState: sealed + Send + Sync + 'static {
    type Wrap<T: Payload>: Payload;               // GAT
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

```rust
#[derive(Clone, Debug)]
pub struct Policy<Cats = Omitted, Fields = Omitted, Tags = Omitted, Tax = Omitted, Emps = Omitted>
where /* all: FetchState */ {
    pub categories:    Cats::Wrap<Vec<Category>>,
    pub report_fields: Fields::Wrap<Vec<ReportField>>,
    pub tags:          Tags::Wrap<Vec<PolicyTag>>,
    pub tax:           Tax::Wrap<Option<TaxConfig>>,   // Option = "policy has no tax config"
    pub employees:     Emps::Wrap<Vec<PolicyEmployee>>,
}
pub type Policies<..> = HashMap<PolicyId, Policy<..>>;
```

The `Option` inside `tax` is deliberate: it encodes data-dependent
absence (`"tax": {}` on the wire), not request-dependent absence — the
distinction this whole mechanism exists to draw.

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

## Reimbursement: strict vs tolerant 207

`responseCode: 207` means partial success. The silent-partial bug — code
assumes everything was reimbursed and never looks at `skippedReports` —
is closed with the `verbose_results` move applied to error strictness:

```rust
impl Client { pub fn mark_reports_reimbursed(&self, targets: ReimburseTargets) -> ReimburseAction<Strict> }

pub struct Strict; pub struct Tolerant;                 // no public trait needed; two impls only
pub struct ReimburseAction<Mode = Strict>;              // .payment_source(..) on all modes
impl ReimburseAction<Strict> { pub fn tolerate_partial(self) -> ReimburseAction<Tolerant> }

impl IntoFuture for ReimburseAction<Strict>   { type Output = Result<Vec<ReportId>, Error>; }
    // 207 -> Err(Error::PartialSuccess(Box<ReimburseOutcome>)); Ok has no lists to forget to check
impl IntoFuture for ReimburseAction<Tolerant> { type Output = Result<ReimburseOutcome, Error>; }
    // 200 and 207 both Ok
```

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
| Domain Cards Getter (`get`/`domainCardList`) | `DomainClient::card_list()` | `DomainCardListAction` | `Vec<DomainCard>` |
| Policy Creator (`create`/`policy`) | `Client::create_policy(name)` | `CreatePolicyAction` | `CreatedPolicy` |
| Report Creator (`create`/`report`) | `Client::create_report(policy, email, title, expenses)` | `CreateReportAction` | `CreatedReport` |
| Expense Creator (`create`/`expenses`) | `Client::create_expenses(expenses)` | `CreateExpensesAction` | `Vec<CreatedTransaction>` |
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

### `error.rs`

```rust
#[non_exhaustive] pub enum Error {          // thiserror
    Transport(#[from] reqwest::Error),
    RateLimited { retry_after: Option<Duration> },       // HTTP 429 or body code 429
    Api(ApiError),                                       // body responseCode != 200/207, any HTTP status
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
hence `Option`). `PartialSuccess` is the one operation-specific variant;
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
Xlsx, Txt, Pdf, Json, Xml), `SftpConnection` (Clone, Debug, pub fields),
`OnFinish` (Clone, Debug; private enum inside), `ExportReportsAction<F>`.

### `reconciliation.rs`

`ReconciliationScope` (Copy, Eq — Unreported, All), `ReconcileAction<F>`.

### `policy/` (flattened via `pub use` in `mod.rs`)

- `model.rs` — data types shared read/write:
  - `Category` — Clone, Debug, PartialEq, SD. Fields: `name: String`,
    `enabled: bool`, `gl_code`, `payroll_code`, `comment_hint:
    Option<String>`, `are_comments_required: Option<bool>`,
    `max_expense_amount_cents: Option<i64>` (wire `maxExpenseAmount`).
    Builder: `new(name)` (enabled), `.disabled()`, `.gl_code()`,
    `.payroll_code()`, `.require_comments()`, `.comment_hint()`,
    `.max_expense_amount_cents()`.
  - `PolicyTag { name, enabled, gl_code: Option<String> }` — same style.
  - `ReportFieldType` — Copy, Eq, SD lowercase: Formula (read-only —
    getter emits it, updater rejects it), Text, Dropdown, Date.
  - `ReportField { name, field_type, values: Vec<String> }` — getter
    shape, Deserialize only.
  - `ReportFieldDef { name, field_type, values: Vec<ReportFieldValue>,
    default_value: Option<String> }` — updater shape, Serialize only.
    `new(name, type)`, `.values(iter Into<ReportFieldValue>)`,
    `.default_value()`. Values always serialize in object form, which
    satisfies Expensify's "uniformly strings or uniformly objects" rule
    by construction.
  - `ReportFieldValue { name, enabled, external_id }` —
    `new`, `.disabled()`, `.external_id()`, `From<&str>/<String>`.
  - `TaxConfig { name, default: TaxRateId, rates: Vec<TaxRate> }`;
    `TaxRate { name, rate: f64, rate_id: TaxRateId }` — Deserialize.
  - `PolicyRole` — Copy, Eq, SD lowercase: User, Auditor, Admin.
  - `PolicyEmployee { email, role, submits_to, employee_id,
    custom_field_1, custom_field_2 }` — Deserialize.
  - `PolicyPlan` — Copy, Eq, SD lowercase: Team, Corporate (wire key
    `type`; renamed to avoid a third meaning of "type").
  - `PolicySummary { id, name, owner, role, output_currency, plan }` —
    Deserialize.
- `get.rs` — `Payload`, `FetchState` (sealed), `Fetched`, `Omitted`,
  `NotFetched`, `Policy<..>`, `Policies<..>` alias, `GetPoliciesBuilder`,
  `GetPoliciesAction<..>`. See [§ Policy getter](#policy-getter-fetch-flags-as-typestate).
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
  `merge_inline` / `replace_all_inline` (independent levels only — the
  inline form simply has no dependency knob, matching the API) and
  `merge_csv(data, config)` / `replace_all_csv` (the CSV goes in the
  `file` form field); `UpdatePolicyAction` — `.categories()`,
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
- `CreateExpensesAction` — `.employee_email(..)` (doc'd: restricted,
  needs advanced permissions; default = credential owner's account).
- `CreatedTransaction { transaction_id, merchant, created: Date,
  amount_cents, currency }` — Clone, Debug.

### `reports.rs`

- `ExpenseLine` — `new(merchant, date, amount: Money)` only. Deliberately
  narrower than `Expense`: the report-creator job accepts exactly these
  four wire fields, so category/tag/etc. cannot be attached and silently
  dropped.
- `CreateReportAction` — `.report_field(name, value)` (repeatable),
  `.report_fields(&impl Serialize)` (must serialize to a JSON object;
  serialized eagerly, failure surfaces at `.await`). Keys normalized
  (non-alphanumeric → `_`) before sending. Doc'd: job requires
  support-side enablement + domain/policy admin.
  `CreatedReport { report_id, name }`.
- `ReimburseTargets`, `Strict`, `Tolerant`, `ReimburseAction<Mode>`,
  `ReimburseOutcome`, `SkippedReport` — see
  [§ Reimbursement](#reimbursement-strict-vs-tolerant-207).

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
  `FetchUrl { url, user, password }` (`"download"`),
  `Sftp { connection: SftpConnection, filename }` (`"sftp"`). An enum,
  not typestate: three mutually exclusive wire shapes, no sequencing.
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
`governor::DefaultDirectRateLimiter`s (quota 5/10 s and 20/60 s); every
send awaits both. Opt-out is `ClientBuilder::no_rate_limiting()` — the
limiter is process-local, so multi-process deployments sharing one
credential need an external governor and will still see
`Error::RateLimited`, which is why 429 remains a surfaced error rather
than an internal retry. No auto-retry in v1 (see Open questions).

## Dependencies & features

| crate | why |
|---|---|
| `reqwest` (no default features; `rustls-tls`, `charset`, `http2`) | HTTP; `multipart` pulled in only by the deprecated-updater feature |
| `serde` (+derive), `serde_json` | envelope + user-type bounds (`Serialize`/`DeserializeOwned`) |
| `bytes` | zero-copy download payloads; `ExportedFile<Bytes>` default |
| `time` (`serde`, `macros`, `formatting`, `parsing`) | `Date`/`PrimitiveDateTime` in the public API; lighter than chrono |
| `governor` | the two-window rate limiter |
| `thiserror` | error derives |
| dev: `tokio` (macros, rt) | examples/tests only — the lib itself is runtime-agnostic-by-accident (reqwest requires tokio anyway, so no abstraction layer is designed) |

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
  export.rs         # ReportsQuery, ReportState, ExportFormat, OnFinish, SftpConnection, ExportReportsAction
  reconciliation.rs # ReconciliationScope, ReconcileAction
  policy/           # mod, model, get, list, create, update, approvers
  expenses.rs       # Expense, ExpenseTax, CreateExpensesAction, CreatedTransaction
  reports.rs        # ExpenseLine, CreateReportAction, ReimburseTargets/Action/Outcome
  expense_rules.rs
  employees.rs      # advanced updater + deprecated (feature)
  cards.rs          # DomainCard, DomainCardListAction
  wire.rs, limit.rs # private (to be created by the implementer)
```

## Misuses made uncompilable

All verified against the skeleton (temporary `examples/misuse.rs`; error
classes quoted from rustc).

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
6. **ID swaps**: `.report_id(policy_id)` — E0277:
   `ReportId: From<PolicyId>` not satisfied.
7. **Filterless export** (documented 410): `ReportsQuery` has only
   anchored constructors — `ReportsQuery::default()` is E0599.
8. **Silently ignoring partial reimbursement**: strict output is
   `Vec<ReportId>` — `outcome.skipped` is E0609; there is no list to
   forget, and an actual 207 is `Err`. Reaching `ReimburseOutcome`
   requires typing `.tolerate_partial()`.
9. **Any report-status value other than reimbursed**: no status
   parameter exists to hold `"APPROVED"` (E0599 on any invented method).
10. **Rich expense fields on the report creator** (would be silently
    dropped by the four-field wire shape):
    `ExpenseLine::new(..).category("Meals")` — E0599.
11. **Double-selecting a policy field**:
    `.with_tax().with_tax()` — E0599 (`with_tax` exists only while that
    slot is `Omitted`).
12. **`setRequired` shape mismatch on tag CSVs** (scalar vs per-level):
    unrepresentable — `TagCsvConfig::dependent` takes `bool`,
    `::independent` takes an iterator; there is no field to set wrongly.

Runtime-but-loud (not compile errors, by design): destructive tag/
category replacement is spelled `replace_all_*`; clearing a tag approver
is `TagApprover::clear`, not an empty string; every action is
`#[must_use]`.

## Naming divergences from the wire

`snake_case` throughout; beyond mechanical case mapping:

| wire | here | why |
|---|---|---|
| `type` (policy plan) | `PolicyPlan::{Team, Corporate}` | `type` already means job and inputSettings discriminators |
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
  constructors achieve the same unrepresentability with zero type
  parameters and better error messages.
- **`Option<TaxConfig>` elimination** — kept: `{}` on the wire is
  genuine data-dependent absence, exactly what `Option` is for.
- **Generic `WriteModel`-style job enum** — Expensify jobs share nothing
  but the envelope; a unifying enum would be `serde_json::Value` with
  extra steps. Continuity comes from the action pattern instead.
- **Runtime-agnostic abstraction** — reqwest pins tokio; a runtime trait
  would be speculative complexity with one impl.
- **`FromExport` sealing** — left open deliberately: user-side CSV/XML
  markers are the escape hatch's escape hatch.

## Open questions

1. **Downloading a not-yet-rendered export.** Report exports are async
   server-side and the docs don't document the "not ready" response.
   v1 surfaces whatever error comes back; if probing shows a stable
   signal, add `DownloadAction::poll_until_ready(interval, timeout)`.
   Needs a live credential to characterize.
2. **429 auto-retry.** Currently surfaced as `Error::RateLimited` with
   no retry. A `ClientBuilder::retry_rate_limited(max: u32)` knob is a
   natural v1.1 addition — rule on whether it belongs in v1.
3. **Expense-rule responses.** Undocumented; if the creator actually
   returns a `ruleID`, `CreateExpenseRuleAction::Output` should become
   `RuleId` (otherwise users can never call `update_expense_rule`).
   Probe with live credentials.
4. **PDF exports** (`fileExtension: "pdf"`, one file *per report*): the
   interaction with `returnRandomFileName` (one name vs many) is
   undocumented. Excluded from the typed path implicitly (nothing stops
   `.format(Pdf)`, but the single-`ExportedFile` model may be wrong for
   it). May need `Vec<ExportedFile>` output or a documented restriction.
5. **`ExportFormat` default for `Json<_>` templates** — doc'd default is
   Csv for all; auto-defaulting typed-JSON exports to `json` needs an
   associated const on `FromExport` (small trait wart). Rule on whether
   the ergonomic win justifies it.
6. **Rate-limit figures** — if the "50 jobs/minute" page resurfaces,
   confirm whether it's a separate *job-start* budget on top of the
   request budget; the limiter currently models requests only.
