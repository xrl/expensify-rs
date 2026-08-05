//! Private wire layer: envelope assembly and response parsing.
//!
//! Everything Expensify-shaped lives here — job type strings, the camelCase
//! and `*ID` key spellings, the string-typed numbers, the response mirrors.
//! There is no published spec and no changelog upstream, so this is the
//! module expected to rot; keeping it in one file is deliberate.
//!
//! The load-bearing rule: **HTTP 200 does not imply success.** Every JSON
//! response carries its own `responseCode` and that code wins over the status
//! line. The Downloader is the exception, since a successful download body is
//! raw file content rather than an envelope.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::{Date, PrimitiveDateTime};

use crate::Url;
use crate::cards::DomainCard;
use crate::client::Client;
use crate::employees::{
    Employee, EmployeeSource, EmployeeUpdateOutcome, PrimaryPolicyMode, SkippedEmployee,
    UpdateEmployeesAction,
};
use crate::error::{ApiError, ApiErrorKind, DecodeError, Error};
use crate::expense_rules::{CreateExpenseRuleAction, UpdateExpenseRuleAction};
use crate::expenses::{CreateExpensesAction, CreatedTransaction, Expense};
use crate::export::{
    ExportFormat, ExportReportsAction, OnFinish, OnFinishKind, ReportState, ReportsQuery,
    SftpConnection,
};
use crate::file::FileSystem;
use crate::observe::{Exchange, ObservedRequest};
use crate::policy::{
    CreatedPolicy, ListPoliciesAction, PolicyField, PolicyPlan, PolicySummary,
    SetTagApproversAction, TagApprover, TagsSource, UpdateMode, UpdatePolicyAction,
};
use crate::reconciliation::{ReconcileAction, ReconciliationFormat, ReconciliationScope};
use crate::reports::{
    CreateReportAction, CreatedReport, ReimburseOutcome, ReimburseTargets, SkippedReport,
};
use crate::secret::{MaskedUrl, REDACTED, Secret};
use crate::types::{Currency, PolicyId, ReportId, TransactionId};

/// The form field carrying the JSON job description.
const JOB_FIELD: &str = "requestJobDescription";

const DATE: &[BorrowedFormatItem<'_>] = format_description!("[year]-[month]-[day]");
const DATE_TIME_SPACE: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
const DATE_TIME_T: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");

// ---------------------------------------------------------------------------
// formatting helpers
// ---------------------------------------------------------------------------

pub(crate) fn fmt_date(date: Date) -> String {
    date.format(DATE)
        .expect("yyyy-mm-dd is infallible for a valid Date")
}

fn parse_date(raw: &str) -> Option<Date> {
    Date::parse(raw.get(..10)?, DATE).ok()
}

fn parse_date_time(raw: &str) -> Option<PrimitiveDateTime> {
    PrimitiveDateTime::parse(raw, DATE_TIME_SPACE)
        .or_else(|_| PrimitiveDateTime::parse(raw, DATE_TIME_T))
        .ok()
}

/// Expensify uses `""` rather than `null` for absent timestamps, so blank
/// is genuinely "no value" — but an unparseable *non-blank* value means the
/// format moved, and silently answering `None` would hide that from every
/// caller. Same rule as `created_transactions`.
fn optional<T>(
    field: &str,
    raw: Option<String>,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, Error> {
    match raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(value) => parse(value).map(Some).ok_or_else(|| {
            DecodeError::custom(format!("unparseable `{field}` value `{value}`")).into()
        }),
    }
}

fn join<T: AsRef<str>>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|i| i.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join(",")
}

fn report_state(state: ReportState) -> &'static str {
    match state {
        ReportState::Open => "OPEN",
        ReportState::Submitted => "SUBMITTED",
        ReportState::Approved => "APPROVED",
        ReportState::Reimbursed => "REIMBURSED",
        ReportState::Archived => "ARCHIVED",
    }
}

fn export_format(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "csv",
        ExportFormat::Xls => "xls",
        ExportFormat::Xlsx => "xlsx",
        ExportFormat::Txt => "txt",
        ExportFormat::Json => "json",
        ExportFormat::Xml => "xml",
    }
}

fn reconciliation_format(format: ReconciliationFormat) -> &'static str {
    match format {
        ReconciliationFormat::Csv => "csv",
        ReconciliationFormat::Txt => "txt",
        ReconciliationFormat::Json => "json",
        ReconciliationFormat::Xml => "xml",
    }
}

fn file_system(fs: FileSystem) -> &'static str {
    match fs {
        FileSystem::IntegrationServer => "integrationServer",
        FileSystem::Reconciliation => "reconciliation",
    }
}

fn update_mode(mode: UpdateMode) -> &'static str {
    match mode {
        UpdateMode::Merge => "merge",
        UpdateMode::ReplaceAll => "replace",
    }
}

fn policy_plan(plan: &PolicyPlan) -> &str {
    match plan {
        PolicyPlan::Team => "team",
        PolicyPlan::Corporate => "corporate",
        PolicyPlan::Other(raw) => raw,
    }
}

fn primary_policy(mode: PrimaryPolicyMode) -> &'static str {
    match mode {
        PrimaryPolicyMode::None => "none",
        PrimaryPolicyMode::NewEmployees => "new_employees",
        PrimaryPolicyMode::AllEmployees => "all_employees",
    }
}

/// Expensify keys report fields by the label with every non-alphanumeric
/// character replaced by `_`.
pub(crate) fn normalize_report_field_key(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// Insert `key` only when the option is populated; Expensify treats an
/// explicit `null` as a value in several jobs.
macro_rules! opt {
    ($map:expr, $key:expr, $value:expr) => {
        if let Some(value) = &$value {
            $map.insert($key.to_owned(), json!(value));
        }
    };
}

// ---------------------------------------------------------------------------
// request envelope
// ---------------------------------------------------------------------------

/// One outbound job: the `requestJobDescription` object plus whichever of
/// the auxiliary form fields the job uses.
pub(crate) struct JobRequest {
    job: Map<String, Value>,
    /// Merged into `credentials` alongside the partner ID/secret — the
    /// employee updater's feed access lives there.
    credential_extras: Map<String, Value>,
    /// Everything this job carries that an observer may not see verbatim.
    /// `job` holds placeholders pointing here; see [`JobRequest::secret`].
    concealed: Vec<Concealed>,
    /// Distinguishes this job's placeholders from any string a caller could
    /// supply, since caller data and placeholders share one JSON tree.
    nonce: u64,
    template: Option<String>,
    file: Option<String>,
    data: Option<String>,
    #[cfg(feature = "employee-updater-deprecated")]
    multipart_data: Option<Bytes>,
}

impl JobRequest {
    fn new(job_type: &str) -> Self {
        let mut job = Map::new();
        job.insert("type".to_owned(), json!(job_type));
        Self {
            job,
            credential_extras: Map::new(),
            concealed: Vec::new(),
            nonce: NEXT_NONCE.fetch_add(1, Ordering::Relaxed),
            template: None,
            file: None,
            data: None,
            #[cfg(feature = "employee-updater-deprecated")]
            multipart_data: None,
        }
    }

    /// The only way a [`Secret`] reaches the job description.
    ///
    /// It returns a placeholder, not the value: the job tree never holds a
    /// secret, so the rendering an observer sees cannot leak one by omission.
    /// `Secret` implements no `Serialize`, so `json!(a_secret)` does not
    /// compile and this is the path a new secret-bearing field has to take.
    fn secret(&mut self, value: &Secret<String>) -> Value {
        self.conceal(Concealed::Whole(value.clone()))
    }

    /// The same, for a URL Expensify needs whole (`user:pass@` included) but
    /// an observer may only see masked.
    fn masked_url(&mut self, url: &MaskedUrl) -> Value {
        self.conceal(Concealed::Url(url.clone()))
    }

    fn conceal(&mut self, value: Concealed) -> Value {
        self.concealed.push(value);
        Value::String(format!(
            "{PLACEHOLDER}{}:{}{PLACEHOLDER}",
            self.nonce,
            self.concealed.len() - 1
        ))
    }

    fn set(mut self, key: &str, value: Value) -> Self {
        self.job.insert(key.to_owned(), value);
        self
    }

    fn input_settings(self, settings: Map<String, Value>) -> Self {
        self.set("inputSettings", Value::Object(settings))
    }

    fn template(mut self, source: &str) -> Self {
        self.template = Some(source.to_owned());
        self
    }

    /// Tag CSV/TSV payloads ride in a urlencoded form field, so they must be
    /// text; non-UTF-8 input is replaced rather than rejected.
    fn file(mut self, data: &Bytes) -> Self {
        self.file = Some(String::from_utf8_lossy(data).into_owned());
        self
    }

    fn data(mut self, data: String) -> Self {
        self.data = Some(data);
        self
    }

    /// The job description as it will be sent, minus credentials. Exposed for
    /// serialization-shape tests.
    #[cfg(test)]
    pub(crate) fn description(&self) -> &Map<String, Value> {
        &self.job
    }

    /// The job description with secrets substituted in, as it goes out.
    #[cfg(test)]
    pub(crate) fn wire_job(&self) -> Value {
        self.render_for(&self.job, Render::Wire)
    }

    /// The same, as an observer would see it.
    #[cfg(test)]
    pub(crate) fn observed_job(&self) -> Value {
        self.render_for(&self.job, Render::Observed)
    }

    #[cfg(test)]
    pub(crate) fn wire_credential_extras(&self) -> Value {
        self.render_for(&self.credential_extras, Render::Wire)
    }

    #[cfg(test)]
    fn render_for(&self, map: &Map<String, Value>, mode: Render) -> Value {
        render(
            &Value::Object(map.clone()),
            &self.concealed,
            self.nonce,
            mode,
        )
    }

    #[cfg(test)]
    pub(crate) fn input(&self) -> &Map<String, Value> {
        self.job["inputSettings"]
            .as_object()
            .expect("inputSettings is an object")
    }

    #[cfg(test)]
    pub(crate) fn template_source(&self) -> Option<&str> {
        self.template.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn file_field(&self) -> Option<&str> {
        self.file.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn data_field(&self) -> Option<&str> {
        self.data.as_deref()
    }

    /// Seal the job: attach credentials and render the form fields.
    ///
    /// `observe` decides whether the second, redacted rendering is produced at
    /// all — nobody watching means no extra tree walk and no extra string.
    fn finish(mut self, client: &Client, observe: bool) -> Rendered {
        let partner_secret = self.secret(&client.inner.credentials.partner_user_secret);
        let mut credentials = std::mem::take(&mut self.credential_extras);
        credentials.insert(
            "partnerUserID".to_owned(),
            json!(client.inner.credentials.partner_user_id),
        );
        credentials.insert("partnerUserSecret".to_owned(), partner_secret);
        self.job
            .insert("credentials".to_owned(), Value::Object(credentials));

        let job_type = self
            .job
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let job = Value::Object(self.job);

        let mut extras: Vec<(&'static str, String)> = Vec::new();
        if let Some(template) = self.template {
            extras.push(("template", template));
        }
        if let Some(file) = self.file {
            extras.push(("file", file));
        }
        if let Some(data) = self.data {
            extras.push(("data", data));
        }

        Rendered {
            job_type,
            description: render(&job, &self.concealed, self.nonce, Render::Wire).to_string(),
            observed_description: observe
                .then(|| render(&job, &self.concealed, self.nonce, Render::Observed).to_string()),
            extras,
        }
    }
}

/// A job description rendered for sending, and — when someone is watching —
/// again with its secrets replaced.
struct Rendered {
    job_type: String,
    description: String,
    observed_description: Option<String>,
    /// `template` / `file` / `data`. No secret rides in any of them.
    extras: Vec<(&'static str, String)>,
}

impl Rendered {
    fn fields(&self) -> Vec<(&str, &str)> {
        let mut fields = vec![(JOB_FIELD, self.description.as_str())];
        fields.extend(self.extras.iter().map(|(k, v)| (*k, v.as_str())));
        fields
    }

    fn observed(&self, url: &Url) -> Option<ObservedRequest> {
        let description = self.observed_description.clone()?;
        let mut fields = vec![(JOB_FIELD, description)];
        fields.extend(self.extras.iter().cloned());
        Some(ObservedRequest::new(
            MaskedUrl::from(url),
            self.job_type.clone(),
            fields,
        ))
    }
}

/// Delimits a secret placeholder. A control character on both ends: caller
/// data that collides with this is not reachable through any documented
/// Expensify field, and the nonce makes a collision unguessable anyway.
const PLACEHOLDER: &str = "\u{1}expensify-secret\u{1}";

static NEXT_NONCE: AtomicU64 = AtomicU64::new(0);

/// A value the wire needs whole and an observer may not have.
enum Concealed {
    /// Nothing about it is printable.
    Whole(Secret<String>),
    /// Only the `user:pass@` is; the host and path are the diagnosis.
    Url(MaskedUrl),
}

impl Concealed {
    fn wire(&self) -> String {
        match self {
            Self::Whole(secret) => secret.expose().clone(),
            Self::Url(url) => url.expose().to_owned(),
        }
    }

    fn observed(&self) -> String {
        match self {
            Self::Whole(_) => REDACTED.to_owned(),
            Self::Url(url) => url.masked(),
        }
    }
}

#[derive(Clone, Copy)]
enum Render {
    /// Substitute the real values — the body that goes out.
    Wire,
    /// Substitute the redacted forms — the body an observer may see.
    Observed,
}

fn render(value: &Value, concealed: &[Concealed], nonce: u64, mode: Render) -> Value {
    match value {
        Value::String(raw) => match slot(raw, nonce).and_then(|index| concealed.get(index)) {
            Some(hidden) => Value::String(match mode {
                Render::Wire => hidden.wire(),
                Render::Observed => hidden.observed(),
            }),
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render(item, concealed, nonce, mode))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), render(item, concealed, nonce, mode)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn slot(raw: &str, nonce: u64) -> Option<usize> {
    let body = raw.strip_prefix(PLACEHOLDER)?.strip_suffix(PLACEHOLDER)?;
    let (found, index) = body.split_once(':')?;
    if found.parse::<u64>().ok()? != nonce {
        return None;
    }
    index.parse().ok()
}

impl Client {
    /// Submit a job and return the parsed success envelope.
    pub(crate) async fn send(&self, request: JobRequest) -> Result<Value, Error> {
        let (status, headers, body) = self.raw(request).await?;
        parse_envelope(status, &headers, &body)
    }

    /// Submit a job whose success body is a file rather than an envelope.
    pub(crate) async fn send_download(&self, request: JobRequest) -> Result<Bytes, Error> {
        let (status, headers, body) = self.raw(request).await?;
        parse_download(status, &headers, body)
    }

    /// Submit a job whose success body is a bare filename rather than an
    /// envelope. See [`parse_filename`].
    pub(crate) async fn send_filename(&self, request: JobRequest) -> Result<String, Error> {
        let (status, headers, body) = self.raw(request).await?;
        parse_filename(status, &headers, &body)
    }

    async fn raw(&self, request: JobRequest) -> Result<(StatusCode, HeaderMap, Bytes), Error> {
        if let Some(gate) = &self.inner.limiter {
            gate.acquire().await;
        }

        #[cfg(feature = "employee-updater-deprecated")]
        let multipart = request.multipart_data.clone();

        let observer = self.inner.observer.clone();
        let rendered = request.finish(self, observer.is_some());
        let observed = observer
            .as_ref()
            .and_then(|_| rendered.observed(&self.inner.base_url));
        if let (Some(observer), Some(request)) = (&observer, &observed) {
            observer.on_request(request);
        }
        // Started after the rate-limit wait, so the duration is the server's
        // and not this crate's.
        let started = observed.as_ref().map(|_| Instant::now());

        let builder = self.inner.http.post(self.inner.base_url.clone());

        #[cfg(feature = "employee-updater-deprecated")]
        let builder = match multipart {
            Some(csv) => {
                let form = reqwest::multipart::Form::new()
                    .text(JOB_FIELD, rendered.description.clone())
                    .part(
                        "data",
                        reqwest::multipart::Part::stream(csv).file_name("employees.csv"),
                    );
                builder.multipart(form)
            }
            None => builder.form(&rendered.fields()),
        };
        #[cfg(not(feature = "employee-updater-deprecated"))]
        let builder = builder.form(&rendered.fields());

        let response = builder.send().await?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await?;

        if let (Some(observer), Some(request), Some(started)) = (observer, observed, started) {
            observer.on_exchange(&Exchange::new(
                request,
                status,
                content_type(&headers),
                body.clone(),
                started.elapsed(),
            ));
        }
        Ok((status, headers, body))
    }
}

fn content_type(headers: &HeaderMap) -> Option<String> {
    headers
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()
        .map(str::to_owned)
}

// ---------------------------------------------------------------------------
// response envelope
// ---------------------------------------------------------------------------

/// Numeric or string on the wire, and JSON has no integer type — a
/// serializer that emits `200.0` still means 200.
fn response_code(map: &Map<String, Value>) -> Option<u16> {
    match map.get("responseCode")? {
        Value::Number(n) => match n.as_u64() {
            Some(code) => code.try_into().ok(),
            None => {
                let float = n.as_f64()?;
                (float.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(&float))
                    .then_some(float as u16)
            }
        },
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn response_message(map: &Map<String, Value>) -> Option<String> {
    map.get("responseMessage")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn error_kind(code: u16) -> ApiErrorKind {
    match code {
        403 => ApiErrorKind::InvalidPermissions,
        404 => ApiErrorKind::NotFound,
        410 => ApiErrorKind::Validation,
        500 => ApiErrorKind::Server,
        _ => ApiErrorKind::Other,
    }
}

fn retry_after(headers: &HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

fn api_error(code: u16, map: &Map<String, Value>, headers: &HeaderMap) -> Error {
    if code == 429 {
        return Error::RateLimited {
            retry_after: retry_after(headers),
        };
    }
    Error::Api(ApiError {
        kind: error_kind(code),
        code,
        message: response_message(map),
    })
}

/// Body `responseCode` first, HTTP status only as a fallback.
fn parse_envelope(status: StatusCode, headers: &HeaderMap, body: &Bytes) -> Result<Value, Error> {
    let parsed = serde_json::from_slice::<Value>(body);
    if let Ok(Value::Object(map)) = &parsed
        && let Some(code) = response_code(map)
    {
        return match code {
            200 | 207 => Ok(parsed.expect("checked above")),
            code => Err(api_error(code, map, headers)),
        };
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::RateLimited {
            retry_after: retry_after(headers),
        });
    }
    if !status.is_success() {
        return Err(Error::Http {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        });
    }
    match parsed {
        Err(err) => Err(DecodeError::Json(err).into()),
        Ok(_) => Err(DecodeError::custom("response was not an Expensify envelope").into()),
    }
}

/// A successful download body is the file itself, so discrimination is by
/// shape rather than by status.
///
/// A JSON *object* is an envelope unless it proves otherwise: a `200` code
/// is treated as content (a template emitting `{"responseCode": 200, ...}`
/// is the disclosed ambiguity), any other code is that error, and an object
/// carrying `responseMessage` with no code at all — Expensify's shape for
/// "File not found" — is an error too, never a caller's export.
///
/// An empty body under HTTP 200 is also an error: a zero-byte export is
/// never useful, and it is the most likely shape of the undocumented
/// "not rendered yet" response. See DESIGN.md open question 1.
fn parse_download(status: StatusCode, headers: &HeaderMap, body: Bytes) -> Result<Bytes, Error> {
    if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&body) {
        match response_code(&map) {
            Some(200) => {}
            Some(code) => return Err(api_error(code, &map, headers)),
            None if map.contains_key("responseMessage") => {
                return Err(DecodeError::custom(format!(
                    "download returned an error envelope: {}",
                    response_message(&map).unwrap_or_default()
                ))
                .into());
            }
            None => {}
        }
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::RateLimited {
            retry_after: retry_after(headers),
        });
    }
    if !status.is_success() {
        return Err(Error::Http {
            status,
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }
    if body.is_empty() {
        return Err(DecodeError::custom(
            "download returned an empty body; the export may not have finished rendering",
        )
        .into());
    }
    Ok(body)
}

/// A submitted Report Exporter answers the generated filename as a **bare
/// body**, not as a JSON envelope:
///
/// ```text
/// HTTP 200, content-type: text/plain;charset=utf-8
/// export0fd99e06-a636-4974-b6bc-3ceb12163386.csv
/// ```
///
/// Observed live 2026-08-04, and the reason `export reports` never worked:
/// the `{"responseCode":200,"filename":…}` shape in Expensify's docs is
/// reconciliation's, and was generalized to the exporter on the assumption
/// that one envelope covers every job.
///
/// **Discrimination is by shape, not by content type.** The same endpoint
/// answers JSON as `text/plain;charset=utf-8` (expense rules, reimburse) and
/// as `application/json` (policy creator), so the header says nothing about
/// the body. What is keyed on instead is the job: only the exporter takes
/// this path, and within it a JSON object is an envelope (an error, or the
/// documented shape if Expensify ever starts sending it) while anything else
/// is the filename itself.
///
/// The bare form is accepted only if it looks like a filename — non-empty and
/// free of control characters. A body that is neither an envelope nor a
/// plausible name (an HTML error page from a proxy, say) is a decode error
/// rather than a handle that would fail later, from `download`, for reasons
/// that no longer mention the export.
fn parse_filename(status: StatusCode, headers: &HeaderMap, body: &Bytes) -> Result<String, Error> {
    if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) {
        match response_code(&map) {
            Some(200) => return filename(Value::Object(map)),
            Some(code) => return Err(api_error(code, &map, headers)),
            None if map.contains_key("responseMessage") => {
                return Err(DecodeError::custom(format!(
                    "export returned an error envelope: {}",
                    response_message(&map).unwrap_or_default()
                ))
                .into());
            }
            None => {}
        }
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::RateLimited {
            retry_after: retry_after(headers),
        });
    }
    if !status.is_success() {
        return Err(Error::Http {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        });
    }

    let name = String::from_utf8_lossy(body).trim().to_owned();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(DecodeError::custom(format!(
            "export response is neither an envelope nor a filename: `{}`",
            String::from_utf8_lossy(body).escape_debug()
        ))
        .into());
    }
    Ok(name)
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|err| DecodeError::Json(err).into())
}

fn missing(field: &str) -> Error {
    DecodeError::custom(format!("response is missing `{field}`")).into()
}

fn take(value: &mut Value, field: &str) -> Option<Value> {
    value.as_object_mut()?.remove(field)
}

fn take_required(value: &mut Value, field: &str) -> Result<Value, Error> {
    take(value, field).ok_or_else(|| missing(field))
}

/// The generated filename out of an envelope. Reconciliation's documented
/// shape, and the exporter's fallback if it ever answers one; the documented
/// key is `filename`, but tolerate the camelCase spelling.
pub(crate) fn filename(mut value: Value) -> Result<String, Error> {
    let raw = take(&mut value, "filename")
        .or_else(|| take(&mut value, "fileName"))
        .ok_or_else(|| missing("filename"))?;
    decode(raw)
}

// ---------------------------------------------------------------------------
// exports
// ---------------------------------------------------------------------------

fn filters(query: &ReportsQuery) -> Map<String, Value> {
    let mut filters = Map::new();
    if !query.report_ids.is_empty() {
        filters.insert(
            "reportIDList".to_owned(),
            json!(join(query.report_ids.iter().map(ReportId::as_str))),
        );
    }
    if !query.policy_ids.is_empty() {
        filters.insert(
            "policyIDList".to_owned(),
            json!(join(query.policy_ids.iter().map(PolicyId::as_str))),
        );
    }
    if let Some(start) = query.start_date {
        filters.insert("startDate".to_owned(), json!(fmt_date(start)));
    }
    if let Some(end) = query.end_date {
        filters.insert("endDate".to_owned(), json!(fmt_date(end)));
    }
    if let Some(approved) = query.approved_after {
        filters.insert("approvedAfter".to_owned(), json!(fmt_date(approved)));
    }
    opt!(filters, "markedAsExported", query.marked_as_exported);
    filters
}

fn sftp_data(request: &mut JobRequest, connection: &SftpConnection) -> Value {
    let password = request.secret(&connection.password);
    json!({
        "host": connection.host,
        "login": connection.login,
        "password": password,
        "port": connection.port,
    })
}

fn on_finish(request: &mut JobRequest, action: &OnFinish) -> Value {
    match &action.kind {
        OnFinishKind::MarkAsExported { label } => {
            json!({ "actionName": "markAsExported", "label": label })
        }
        OnFinishKind::Email {
            recipients,
            message,
        } => {
            let mut map = Map::new();
            map.insert("actionName".to_owned(), json!("email"));
            map.insert("recipients".to_owned(), json!(recipients));
            opt!(map, "message", message);
            Value::Object(map)
        }
        OnFinishKind::SftpUpload(connection) => json!({
            "actionName": "sftpUpload",
            "sftpData": sftp_data(request, connection),
        }),
    }
}

pub(crate) fn export_reports<F>(action: &ExportReportsAction<F>) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("combinedReportData"));
    if !action.states.is_empty() {
        input.insert(
            "reportState".to_owned(),
            json!(join(action.states.iter().copied().map(report_state))),
        );
    }
    // `limit` is a string on the wire even though it is numeric.
    if let Some(limit) = action.limit {
        input.insert("limit".to_owned(), json!(limit.to_string()));
    }
    opt!(input, "employeeEmail", action.employee_email);
    input.insert("filters".to_owned(), Value::Object(filters(&action.query)));

    let mut output = Map::new();
    output.insert(
        "fileExtension".to_owned(),
        json!(export_format(action.format.unwrap_or(ExportFormat::Csv))),
    );
    opt!(output, "fileBasename", action.file_basename);

    let mut request = JobRequest::new("file")
        .set(
            "onReceive",
            json!({ "immediateResponse": ["returnRandomFileName"] }),
        )
        .input_settings(input)
        .set("outputSettings", Value::Object(output))
        .template(&action.template);

    let mut finishers = Vec::with_capacity(action.on_finish.len());
    for action in &action.on_finish {
        finishers.push(on_finish(&mut request, action));
    }
    if !finishers.is_empty() {
        request = request.set("onFinish", Value::Array(finishers));
    }
    if action.test {
        // The doc's parameter table types `test` as String, not boolean. If
        // a boolean were silently ignored, `.test_run()` would be a no-op
        // and every `onFinish` — including the irreversible
        // `markAsExported` — would fire during a believed dry run.
        request = request.set("test", json!("true"));
    }
    request
}

pub(crate) fn download(name: &str, fs: FileSystem) -> JobRequest {
    JobRequest::new("download")
        .set("fileName", json!(name))
        .set("fileSystem", json!(file_system(fs)))
}

pub(crate) fn reconcile<F>(action: &ReconcileAction<F>) -> JobRequest {
    let mut input = Map::new();
    input.insert("startDate".to_owned(), json!(fmt_date(action.start)));
    input.insert("endDate".to_owned(), json!(fmt_date(action.end)));
    input.insert("domain".to_owned(), json!(action.domain));
    input.insert(
        "type".to_owned(),
        json!(match action.scope {
            ReconciliationScope::Unreported => "Unreported",
            ReconciliationScope::All => "All",
        }),
    );
    // Only the synchronous mode works upstream, so it is not a parameter.
    input.insert("async".to_owned(), json!(false));
    input.insert(
        "feed".to_owned(),
        json!(action.feed.as_deref().unwrap_or("export_all_feeds")),
    );

    let mut request = JobRequest::new("reconciliation")
        .input_settings(input)
        .set(
            "outputSettings",
            json!({
                "fileExtension":
                    reconciliation_format(action.format.unwrap_or(ReconciliationFormat::Csv))
            }),
        )
        .template(&action.template);

    if let Some(recipients) = &action.email_on_finish {
        request = request.set(
            "onFinish",
            json!([{ "actionName": "email", "recipients": recipients }]),
        );
    }
    request
}

// ---------------------------------------------------------------------------
// policies
// ---------------------------------------------------------------------------

pub(crate) fn list_policies(action: &ListPoliciesAction) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("policyList"));
    if action.admin_only {
        input.insert("adminOnly".to_owned(), json!(true));
    }
    opt!(input, "userEmail", action.user_email);
    JobRequest::new("get").input_settings(input)
}

#[derive(Deserialize)]
struct PolicyListResponse {
    #[serde(rename = "policyList", default)]
    policies: Vec<PolicySummary>,
}

pub(crate) fn policy_list(value: Value) -> Result<Vec<PolicySummary>, Error> {
    Ok(decode::<PolicyListResponse>(value)?.policies)
}

pub(crate) fn get_policies(
    ids: &[PolicyId],
    fields: &[PolicyField],
    user_email: Option<&str>,
) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("policy"));
    input.insert(
        "fields".to_owned(),
        json!(fields.iter().map(|field| field.wire()).collect::<Vec<_>>()),
    );
    input.insert("policyIDList".to_owned(), json!(ids));
    opt!(input, "userEmail", user_email);
    JobRequest::new("get").input_settings(input)
}

pub(crate) fn policy_info(mut value: Value) -> Result<HashMap<PolicyId, Value>, Error> {
    let raw = take_required(&mut value, "policyInfo")?;
    decode(raw)
}

/// The getter answers `"tax": {}` for a policy with no tax configuration.
/// Rewriting that to `null` is what lets the public field stay a plain
/// `Option<TaxConfig>`.
pub(crate) fn normalize_tax(value: Value) -> Value {
    match value {
        Value::Object(map) if map.is_empty() => Value::Null,
        other => other,
    }
}

pub(crate) fn create_policy(name: &str, plan: Option<&PolicyPlan>) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("policy"));
    input.insert("policyName".to_owned(), json!(name));
    if let Some(plan) = plan {
        input.insert("plan".to_owned(), json!(policy_plan(plan)));
    }
    JobRequest::new("create").input_settings(input)
}

#[derive(Deserialize)]
struct CreatePolicyResponse {
    #[serde(rename = "policyID")]
    policy_id: PolicyId,
    #[serde(rename = "policyName")]
    policy_name: String,
}

pub(crate) fn created_policy(value: Value) -> Result<CreatedPolicy, Error> {
    let wire: CreatePolicyResponse = decode(value)?;
    Ok(CreatedPolicy {
        policy_id: wire.policy_id,
        name: wire.policy_name,
    })
}

pub(crate) fn update_policy(action: &UpdatePolicyAction) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("policy"));
    if let [only] = action.policy_ids.as_slice() {
        input.insert("policyID".to_owned(), json!(only));
    } else {
        input.insert("policyIDList".to_owned(), json!(action.policy_ids));
    }

    let mut request = JobRequest::new("update").input_settings(input);

    if let Some(update) = &action.categories {
        request = request.set(
            "categories",
            json!({ "action": update_mode(update.mode), "data": update.data }),
        );
    }
    if let Some(update) = &action.report_fields {
        request = request.set(
            "reportFields",
            json!({ "action": update_mode(update.mode), "data": update.data }),
        );
    }
    if let Some(update) = &action.tags {
        let mut tags = Map::new();
        tags.insert("action".to_owned(), json!(update_mode(update.mode)));
        match &update.source {
            TagsSource::Inline(levels) => {
                tags.insert("source".to_owned(), json!("inline"));
                tags.insert(
                    "data".to_owned(),
                    Value::Array(
                        levels
                            .iter()
                            .map(|level| {
                                let mut map = Map::new();
                                opt!(map, "name", level.name);
                                map.insert("setRequired".to_owned(), json!(level.required));
                                map.insert("tags".to_owned(), json!(level.tags));
                                Value::Object(map)
                            })
                            .collect(),
                    ),
                );
            }
            TagsSource::Csv { data, config } => {
                tags.insert("source".to_owned(), json!("file"));
                tags.insert(
                    "config".to_owned(),
                    json!({
                        "dependency": config.dependent,
                        // Scalar for dependent levels, per-level array otherwise.
                        "setRequired": if config.dependent {
                            json!(config.set_required.first().copied().unwrap_or(false))
                        } else {
                            json!(config.set_required)
                        },
                        "glCodes": config.gl_codes,
                        "header": config.header_row,
                        // Upstream docs say "cvs"; the working value is "csv".
                        "fileType": if config.tsv { "tsv" } else { "csv" },
                    }),
                );
                request = request.file(data);
            }
        }
        request = request.set("tags", Value::Object(tags));
    }
    request
}

pub(crate) fn set_tag_approvers(action: &SetTagApproversAction) -> JobRequest {
    JobRequest::new("update")
        .input_settings({
            let mut input = Map::new();
            input.insert("type".to_owned(), json!("tagApprovers"));
            input.insert("policyID".to_owned(), json!(action.policy_id));
            input
        })
        .set(
            "tagApprovers",
            Value::Array(
                action
                    .approvers
                    .iter()
                    .map(|approver: &TagApprover| {
                        // `""` is Expensify's "unassign" sentinel.
                        json!({
                            "name": approver.name,
                            "approver": approver.approver.as_deref().unwrap_or(""),
                        })
                    })
                    .collect(),
            ),
        )
}

// ---------------------------------------------------------------------------
// reports & expenses
// ---------------------------------------------------------------------------

pub(crate) fn create_report(action: &CreateReportAction) -> JobRequest {
    let mut report = Map::new();
    report.insert("title".to_owned(), json!(action.title));
    if !action.fields.is_empty() {
        report.insert(
            "fields".to_owned(),
            Value::Object(
                action
                    .fields
                    .iter()
                    .map(|(key, value)| (normalize_report_field_key(key), value.clone()))
                    .collect(),
            ),
        );
    }

    let mut input = Map::new();
    input.insert("type".to_owned(), json!("report"));
    input.insert("policyID".to_owned(), json!(action.policy_id));
    input.insert("employeeEmail".to_owned(), json!(action.employee_email));
    input.insert("report".to_owned(), Value::Object(report));
    input.insert(
        "expenses".to_owned(),
        Value::Array(
            action
                .expenses
                .iter()
                .map(|line| {
                    // The report creator spells the date `date`; the expense
                    // creator spells the same thing `created`.
                    json!({
                        "date": fmt_date(line.date),
                        "merchant": line.merchant,
                        "amount": line.amount.cents,
                        "currency": line.amount.currency,
                    })
                })
                .collect(),
        ),
    );
    JobRequest::new("create").input_settings(input)
}

#[derive(Deserialize)]
struct CreateReportResponse {
    #[serde(rename = "reportID")]
    report_id: ReportId,
    #[serde(rename = "reportName")]
    report_name: String,
}

pub(crate) fn created_report(value: Value) -> Result<CreatedReport, Error> {
    let wire: CreateReportResponse = decode(value)?;
    Ok(CreatedReport {
        report_id: wire.report_id,
        name: wire.report_name,
    })
}

fn transaction(expense: &Expense) -> Value {
    let mut map = Map::new();
    map.insert("merchant".to_owned(), json!(expense.merchant));
    map.insert("created".to_owned(), json!(fmt_date(expense.date)));
    map.insert("amount".to_owned(), json!(expense.amount.cents));
    map.insert("currency".to_owned(), json!(expense.amount.currency));
    opt!(map, "externalID", expense.external_id);
    opt!(map, "category", expense.category);
    opt!(map, "tag", expense.tag);
    opt!(map, "billable", expense.billable);
    opt!(map, "reimbursable", expense.reimbursable);
    opt!(map, "comment", expense.comment);
    opt!(map, "reportID", expense.report_id);
    opt!(map, "policyID", expense.policy_id);
    if let Some(tax) = &expense.tax {
        let mut tax_map = Map::new();
        tax_map.insert("rateID".to_owned(), json!(tax.rate_id));
        opt!(tax_map, "amount", tax.amount_cents);
        map.insert("tax".to_owned(), Value::Object(tax_map));
    }
    Value::Object(map)
}

pub(crate) fn create_expenses(action: &CreateExpensesAction) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("expenses"));
    // Required, with or without a policy: omitting it is a 410, `'employeeEmail'
    // parameter is missing or malformed` (observed live 2026-08-04).
    input.insert("employeeEmail".to_owned(), json!(action.employee_email));
    input.insert(
        "transactionList".to_owned(),
        Value::Array(action.expenses.iter().map(transaction).collect()),
    );
    JobRequest::new("create").input_settings(input)
}

#[derive(Deserialize)]
struct TransactionListResponse {
    #[serde(rename = "transactionList", default)]
    transactions: Vec<TransactionWire>,
}

#[derive(Deserialize)]
struct TransactionWire {
    #[serde(rename = "transactionID")]
    transaction_id: TransactionId,
    merchant: String,
    created: String,
    amount: i64,
    currency: Currency,
}

pub(crate) fn created_transactions(value: Value) -> Result<Vec<CreatedTransaction>, Error> {
    decode::<TransactionListResponse>(value)?
        .transactions
        .into_iter()
        .map(|wire| {
            Ok(CreatedTransaction {
                transaction_id: wire.transaction_id,
                merchant: wire.merchant,
                created: parse_date(&wire.created).ok_or_else(|| {
                    Error::from(DecodeError::custom(format!(
                        "unparseable transaction date `{}`",
                        wire.created
                    )))
                })?,
                amount_cents: wire.amount,
                currency: wire.currency,
            })
        })
        .collect()
}

pub(crate) fn reimburse(targets: &ReimburseTargets, payment_source: Option<&str>) -> JobRequest {
    let mut filters = Map::new();
    if !targets.report_ids.is_empty() {
        filters.insert(
            "reportIDList".to_owned(),
            json!(join(targets.report_ids.iter().map(ReportId::as_str))),
        );
    }
    if let Some(start) = targets.start_date {
        filters.insert("startDate".to_owned(), json!(fmt_date(start)));
    }
    if let Some(end) = targets.end_date {
        filters.insert("endDate".to_owned(), json!(fmt_date(end)));
    }

    let mut input = Map::new();
    input.insert("type".to_owned(), json!("reportStatus"));
    // REIMBURSED is the only value Expensify accepts.
    input.insert("status".to_owned(), json!("REIMBURSED"));
    opt!(input, "paymentSource", payment_source);
    input.insert("filters".to_owned(), Value::Object(filters));
    JobRequest::new("update").input_settings(input)
}

#[derive(Deserialize)]
struct ReimburseResponse {
    #[serde(rename = "reportIDs", default)]
    updated: Vec<ReportId>,
    #[serde(rename = "skippedReports", default)]
    skipped: Vec<SkippedReportWire>,
    #[serde(rename = "failedReports", default)]
    failed: Vec<SkippedReportWire>,
}

#[derive(Deserialize)]
struct SkippedReportWire {
    #[serde(rename = "reportID")]
    report_id: ReportId,
    #[serde(default)]
    reason: String,
}

impl From<SkippedReportWire> for SkippedReport {
    fn from(wire: SkippedReportWire) -> Self {
        Self {
            report_id: wire.report_id,
            reason: wire.reason,
        }
    }
}

pub(crate) fn reimburse_outcome(value: Value) -> Result<ReimburseOutcome, Error> {
    let wire: ReimburseResponse = decode(value)?;
    Ok(ReimburseOutcome {
        updated: wire.updated,
        skipped: wire.skipped.into_iter().map(Into::into).collect(),
        failed: wire.failed.into_iter().map(Into::into).collect(),
    })
}

/// 207 is the documented partial-success code. It is *a* signal, not the
/// signal: a run that skipped every report came back 200, so the strict path
/// also checks the skipped/failed lists. See [`crate::ReimburseAction`].
pub(crate) fn is_partial(value: &Value) -> bool {
    value
        .as_object()
        .and_then(response_code)
        .is_some_and(|code| code == 207)
}

// ---------------------------------------------------------------------------
// expense rules
// ---------------------------------------------------------------------------

fn rule_actions(tag: Option<&String>, default_billable: Option<bool>) -> Value {
    let mut actions = Map::new();
    opt!(actions, "tag", tag);
    opt!(actions, "defaultBillable", default_billable);
    Value::Object(actions)
}

pub(crate) fn create_expense_rule(action: &CreateExpenseRuleAction) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("expenseRules"));
    input.insert("policyID".to_owned(), json!(action.policy_id));
    input.insert("employeeEmail".to_owned(), json!(action.employee_email));
    input.insert(
        "actions".to_owned(),
        rule_actions(action.tag.as_ref(), action.default_billable),
    );
    JobRequest::new("create").input_settings(input)
}

pub(crate) fn update_expense_rule(action: &UpdateExpenseRuleAction) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("expenseRules"));
    input.insert("policyID".to_owned(), json!(action.policy_id));
    input.insert("employeeEmail".to_owned(), json!(action.employee_email));
    input.insert("ruleID".to_owned(), json!(action.rule_id));
    input.insert(
        "actions".to_owned(),
        rule_actions(action.tag.as_ref(), action.default_billable),
    );
    JobRequest::new("update").input_settings(input)
}

// ---------------------------------------------------------------------------
// employees
// ---------------------------------------------------------------------------

fn employee(employee: &Employee) -> Value {
    let mut map = Map::new();
    map.insert("employeeEmail".to_owned(), json!(employee.employee_email));
    map.insert("managerEmail".to_owned(), json!(employee.manager_email));
    map.insert("employeeID".to_owned(), json!(employee.employee_id));
    map.insert("policyID".to_owned(), json!(employee.policy_id));
    opt!(map, "firstName", employee.first_name);
    opt!(map, "lastName", employee.last_name);
    opt!(map, "customField1", employee.custom_field_1);
    opt!(map, "customField2", employee.custom_field_2);
    opt!(map, "approvalLimit", employee.approval_limit);
    opt!(map, "overLimitApprover", employee.over_limit_approver);
    opt!(map, "workerStatus", employee.worker_status);
    opt!(map, "isTerminated", employee.is_terminated);
    opt!(map, "domainGroupID", employee.domain_group_id);
    opt!(map, "approvesTo", employee.approves_to);
    opt!(map, "role", employee.role);
    if !employee.additional_policy_ids.is_empty() {
        map.insert(
            "additionalPolicyIDs".to_owned(),
            json!(employee.additional_policy_ids),
        );
    }
    if employee.remove_from_unassigned_policies {
        map.insert("shouldRemoveFromUnassignedPolicies".to_owned(), json!(true));
    }
    if !employee.default_tags.is_empty() {
        map.insert("defaultTags".to_owned(), json!(employee.default_tags));
    }
    Value::Object(map)
}

pub(crate) fn update_employees(action: &UpdateEmployeesAction) -> JobRequest {
    let mut request = JobRequest::new("update")
        .input_settings({
            let mut input = Map::new();
            input.insert("type".to_owned(), json!("employees"));
            input.insert("entity".to_owned(), json!("generic"));
            input
        })
        // Hyphenated on the wire; the only such key in the API.
        .set("dry-run", json!(action.dry_run))
        .set("shouldFixApprovalChains", json!(action.fix_approval_chains));

    if let Some(mode) = action.primary_policy {
        request = request.set("setEmployeePrimaryPolicy", json!(primary_policy(mode)));
    }
    if action.first_level_managers_only {
        request = request.set("fixFirstLevelManagersOnly", json!(true));
    }
    if action.skip_notification_emails {
        request = request.set("shouldSkipNotificationEmail", json!(true));
    }
    if let Some(recipients) = &action.email_on_finish {
        request = request.set(
            "onFinish",
            json!([{ "actionName": "email", "recipients": recipients }]),
        );
    }

    match &action.source {
        EmployeeSource::Inline(employees) => {
            let feed = Value::Array(employees.iter().map(employee).collect());
            request = request
                .set("dataSource", json!("request"))
                .data(feed.to_string());
        }
        EmployeeSource::FetchUrl {
            url,
            user,
            password,
        } => {
            request = request.set("dataSource", json!("download"));
            let feed_url = request.masked_url(url);
            request
                .credential_extras
                .insert("feedUrl".to_owned(), feed_url);
            opt!(request.credential_extras, "feedUser", user);
            if let Some(password) = password {
                let slot = request.secret(password);
                request
                    .credential_extras
                    .insert("feedPassword".to_owned(), slot);
            }
        }
        EmployeeSource::Sftp {
            connection,
            filename,
        } => {
            request = request.set("dataSource", json!("sftp"));
            let mut sftp = sftp_data(&mut request, connection);
            if let Some(map) = sftp.as_object_mut() {
                map.insert("filename".to_owned(), json!(filename));
            }
            request.credential_extras.insert("sftp".to_owned(), sftp);
        }
    }
    request
}

#[derive(Deserialize)]
struct EmployeeUpdateResponse {
    #[serde(rename = "dry-run", default)]
    dry_run: bool,
    #[serde(rename = "updatedEmployeesCount", default)]
    updated_count: u64,
    #[serde(default)]
    diff: EmployeeDiff,
    #[serde(rename = "securityGroupEmployeesMap", default)]
    security_groups: HashMap<String, Vec<String>>,
    #[serde(rename = "skippedEmployees", default)]
    skipped: Vec<SkippedEmployeeWire>,
}

#[derive(Default, Deserialize)]
struct EmployeeDiff {
    #[serde(rename = "diffToAdd", default)]
    add: HashMap<PolicyId, Vec<String>>,
    #[serde(rename = "diffToRemove", default)]
    remove: HashMap<PolicyId, Vec<String>>,
}

#[derive(Deserialize)]
struct SkippedEmployeeWire {
    #[serde(default)]
    email: String,
    #[serde(default)]
    reason: String,
}

pub(crate) fn employee_outcome(value: Value) -> Result<EmployeeUpdateOutcome, Error> {
    let wire: EmployeeUpdateResponse = decode(value)?;
    Ok(EmployeeUpdateOutcome {
        dry_run: wire.dry_run,
        updated_count: wire.updated_count,
        added: wire.diff.add,
        removed: wire.diff.remove,
        security_group_assignments: wire.security_groups,
        skipped: wire
            .skipped
            .into_iter()
            .map(|s| SkippedEmployee {
                email: s.email,
                reason: s.reason,
            })
            .collect(),
    })
}

#[cfg(feature = "employee-updater-deprecated")]
pub(crate) fn update_employees_csv(policy_id: &PolicyId, csv: Bytes) -> JobRequest {
    let mut request = JobRequest::new("update").input_settings({
        let mut input = Map::new();
        input.insert("type".to_owned(), json!("employees"));
        input.insert("policyID".to_owned(), json!(policy_id));
        // The only `fileType` in the API that is not part of a tag config.
        input.insert("fileType".to_owned(), json!("csv"));
        input
    });
    request.multipart_data = Some(csv);
    request
}

#[cfg(feature = "employee-updater-deprecated")]
pub(crate) fn nb_employees(mut value: Value) -> Result<u64, Error> {
    decode(take_required(&mut value, "nbEmployees")?)
}

// ---------------------------------------------------------------------------
// domain cards
// ---------------------------------------------------------------------------

pub(crate) fn card_list(domain: &str) -> JobRequest {
    let mut input = Map::new();
    input.insert("type".to_owned(), json!("domainCardList"));
    input.insert("domain".to_owned(), json!(domain));
    JobRequest::new("get").input_settings(input)
}

#[derive(Deserialize)]
struct DomainCardListResponse {
    #[serde(rename = "domainCardList", default)]
    cards: Vec<DomainCardWire>,
}

#[derive(Deserialize)]
struct DomainCardWire {
    #[serde(default)]
    bank: String,
    #[serde(rename = "cardID", default)]
    card_id: i64,
    #[serde(rename = "cardName", default)]
    card_name: String,
    #[serde(rename = "cardNumber", default)]
    card_number: String,
    #[serde(default)]
    email: String,
    #[serde(rename = "externalEmployeeID", default)]
    external_employee_id: Option<String>,
    #[serde(default)]
    created: Option<String>,
    #[serde(rename = "lastImport", default)]
    last_import: Option<String>,
    #[serde(rename = "lastImportResult", default)]
    last_import_result: Option<u16>,
    #[serde(default)]
    reimbursable: bool,
    #[serde(rename = "scrapeMinDate", default)]
    scrape_min_date: Option<String>,
}

/// Expensify uses `""` rather than `null` for absent card fields.
fn blank_to_none(raw: Option<String>) -> Option<String> {
    raw.filter(|s| !s.trim().is_empty())
}

pub(crate) fn domain_cards(value: Value) -> Result<Vec<DomainCard>, Error> {
    decode::<DomainCardListResponse>(value)?
        .cards
        .into_iter()
        .map(|wire| {
            Ok(DomainCard {
                bank: wire.bank,
                card_id: wire.card_id,
                card_name: wire.card_name,
                card_number: wire.card_number,
                email: wire.email,
                external_employee_id: blank_to_none(wire.external_employee_id),
                created: optional("created", wire.created, parse_date_time)?,
                last_import: optional("lastImport", wire.last_import, parse_date_time)?,
                last_import_result: wire.last_import_result,
                reimbursable: wire.reimbursable,
                // A full datetime upstream; only the date survives.
                scrape_min_date: optional("scrapeMinDate", wire.scrape_min_date, parse_date)?,
            })
        })
        .collect()
}

#[cfg(test)]
// `129_00` reads as dollars-and-cents, which is the point.
#[allow(clippy::inconsistent_digit_grouping)]
mod tests {
    use super::*;
    use crate::expenses::ExpenseTax;
    use crate::policy::{
        Category, PolicyTag, ReportFieldDef, ReportFieldDefType, ReportFieldValue, TagCsvConfig,
        TagLevel, TagsUpdate,
    };
    use crate::reports::ExpenseLine;
    use crate::types::Money;
    use time::macros::date;

    fn envelope(code: u16) -> Bytes {
        Bytes::from(format!(
            r#"{{"responseCode":{code},"responseMessage":"nope"}}"#
        ))
    }

    #[test]
    fn body_code_beats_http_200() {
        let err = parse_envelope(StatusCode::OK, &HeaderMap::new(), &envelope(410)).unwrap_err();
        match err {
            Error::Api(api) => {
                assert_eq!(api.code, 410);
                assert_eq!(api.kind, ApiErrorKind::Validation);
                assert_eq!(api.message.as_deref(), Some("nope"));
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn body_code_429_is_rate_limited() {
        let err = parse_envelope(StatusCode::OK, &HeaderMap::new(), &envelope(429)).unwrap_err();
        assert!(matches!(err, Error::RateLimited { retry_after: None }));
    }

    #[test]
    fn http_429_without_envelope_is_rate_limited() {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "30".parse().unwrap());
        let err = parse_envelope(
            StatusCode::TOO_MANY_REQUESTS,
            &headers,
            &Bytes::from_static(b"slow down"),
        )
        .unwrap_err();
        match err {
            Error::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(std::time::Duration::from_secs(30)));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn non_success_http_without_envelope_is_http_error() {
        let err = parse_envelope(
            StatusCode::BAD_GATEWAY,
            &HeaderMap::new(),
            &Bytes::from_static(b"<html>nginx</html>"),
        )
        .unwrap_err();
        match err {
            Error::Http { status, body } => {
                assert_eq!(status, StatusCode::BAD_GATEWAY);
                assert!(body.contains("nginx"));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn download_body_is_content_not_envelope() {
        let body = Bytes::from_static(b"a,b,c\n1,2,3\n");
        let out = parse_download(StatusCode::OK, &HeaderMap::new(), body.clone()).unwrap();
        assert_eq!(out, body);
    }

    #[test]
    fn download_json_error_envelope_is_failure() {
        let err = parse_download(StatusCode::OK, &HeaderMap::new(), envelope(404)).unwrap_err();
        match err {
            Error::Api(api) => assert_eq!(api.kind, ApiErrorKind::NotFound),
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn download_json_content_survives() {
        // A template emitting JSON must not be mistaken for an envelope.
        let body = Bytes::from_static(br#"[{"report_id":"R1"}]"#);
        let out = parse_download(StatusCode::OK, &HeaderMap::new(), body.clone()).unwrap();
        assert_eq!(out, body);
    }

    #[test]
    fn download_envelope_without_a_code_is_not_content() {
        let body = Bytes::from_static(br#"{"responseMessage":"File not found"}"#);
        let err = parse_download(StatusCode::OK, &HeaderMap::new(), body).unwrap_err();
        match err {
            Error::Decode(DecodeError::Custom(msg)) => assert!(msg.contains("File not found")),
            other => panic!("expected an error envelope, got {other:?}"),
        }
    }

    #[test]
    fn download_empty_body_is_not_success() {
        let err = parse_download(StatusCode::OK, &HeaderMap::new(), Bytes::new()).unwrap_err();
        match err {
            Error::Decode(DecodeError::Custom(msg)) => assert!(msg.contains("empty body")),
            other => panic!("expected a decode error, got {other:?}"),
        }
    }

    /// The exporter's observed success shape. `text/plain` is deliberately
    /// *not* what says so — this endpoint sends JSON under that same header
    /// for other jobs — so the header is absent here and parsing works anyway.
    #[test]
    fn a_bare_body_is_the_exported_filename() {
        let body = Bytes::from_static(b"export0fd99e06-a636-4974-b6bc-3ceb12163386.csv");
        let name = parse_filename(StatusCode::OK, &HeaderMap::new(), &body).unwrap();
        assert_eq!(name, "export0fd99e06-a636-4974-b6bc-3ceb12163386.csv");
    }

    #[test]
    fn an_envelope_is_still_an_envelope_for_the_exporter() {
        let body = Bytes::from_static(br#"{"responseCode":200,"filename":"export_1.csv"}"#);
        assert_eq!(
            parse_filename(StatusCode::OK, &HeaderMap::new(), &body).unwrap(),
            "export_1.csv"
        );

        let failure =
            parse_filename(StatusCode::OK, &HeaderMap::new(), &envelope(410)).unwrap_err();
        match failure {
            Error::Api(api) => assert_eq!(api.code, 410),
            other => panic!("expected Api, got {other:?}"),
        }
    }

    /// A filename is one line of text. Anything else — an empty body, a proxy's
    /// HTML — is a decode error rather than a handle whose download fails later
    /// for reasons that never mention the export.
    #[test]
    fn a_body_that_is_not_a_filename_is_rejected() {
        for body in [
            Bytes::new(),
            Bytes::from_static(b"   \n"),
            Bytes::from_static(b"<html>\n<body>502</body>\n</html>"),
        ] {
            match parse_filename(StatusCode::OK, &HeaderMap::new(), &body) {
                Err(Error::Decode(DecodeError::Custom(msg))) => {
                    assert!(msg.contains("neither an envelope nor a filename"), "{msg}");
                }
                other => panic!("expected a decode error, got {other:?}"),
            }
        }
    }

    /// Live evidence: tags sent with `action: "merge"` deleted every unlisted
    /// tag and answered 200. Whatever `TagsUpdate` grows, `merge` must not
    /// reach the wire for tags.
    #[test]
    fn tag_updates_never_send_merge() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let inline = client
            .update_policy("P1")
            .tags(TagsUpdate::replace_all_inline([TagLevel::new([
                PolicyTag::new("Gamma"),
            ])]));
        let csv = client.update_policy("P1").tags(TagsUpdate::replace_all_csv(
            Bytes::from_static(b"Gamma\n"),
            TagCsvConfig::dependent(false),
        ));
        for action in [inline, csv] {
            assert_eq!(
                update_policy(&action).description()["tags"]["action"],
                "replace"
            );
        }
    }

    #[test]
    fn float_response_codes_are_still_codes() {
        let body = Bytes::from_static(br#"{"responseCode":410.0,"responseMessage":"nope"}"#);
        let err = parse_envelope(StatusCode::OK, &HeaderMap::new(), &body).unwrap_err();
        match err {
            Error::Api(api) => assert_eq!(api.code, 410),
            other => panic!("expected Api, got {other:?}"),
        }
        let ok = Bytes::from_static(br#"{"responseCode":200.0}"#);
        assert!(parse_envelope(StatusCode::OK, &HeaderMap::new(), &ok).is_ok());
    }

    #[test]
    fn unparseable_card_dates_are_errors_but_blanks_are_not() {
        let response = json!({
            "domainCardList": [
                { "cardID": 1, "created": "", "lastImport": "", "scrapeMinDate": "" }
            ]
        });
        let cards = domain_cards(response).unwrap();
        assert!(cards[0].created.is_none());

        let broken = json!({
            "domainCardList": [{ "cardID": 1, "created": "07/31/2026 03:04:05" }]
        });
        match domain_cards(broken).unwrap_err() {
            Error::Decode(DecodeError::Custom(msg)) => assert!(msg.contains("created"), "{msg}"),
            other => panic!("expected a decode error, got {other:?}"),
        }
    }

    #[test]
    fn export_serialization_shape() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let template: crate::ExportTemplate<crate::Json<Vec<u8>>> =
            crate::ExportTemplate::typed("<#list reports as r></#list>");
        let action = client
            .export_reports(
                &template,
                ReportsQuery::since(date!(2026 - 07 - 01))
                    .until(date!(2026 - 08 - 01))
                    .policy_ids(["P1", "P2"])
                    .not_yet_exported_as("acme-etl"),
            )
            .state(ReportState::Approved)
            .state(ReportState::Reimbursed)
            .limit(50)
            .format(ExportFormat::Json)
            .file_basename("close")
            .mark_as_exported("acme-etl")
            .test_run();

        let request = export_reports(&action);
        let input = request.input();

        assert_eq!(input["type"], "combinedReportData");
        assert_eq!(input["reportState"], "APPROVED,REIMBURSED");
        // A string, not a number.
        assert_eq!(input["limit"], json!("50"));
        assert_eq!(input["filters"]["policyIDList"], "P1,P2");
        assert_eq!(input["filters"]["startDate"], "2026-07-01");
        assert_eq!(input["filters"]["endDate"], "2026-08-01");
        assert_eq!(input["filters"]["markedAsExported"], "acme-etl");

        let job = request.description();
        assert_eq!(job["type"], "file");
        assert_eq!(
            job["onReceive"]["immediateResponse"][0],
            "returnRandomFileName"
        );
        assert_eq!(job["outputSettings"]["fileExtension"], "json");
        assert_eq!(job["outputSettings"]["fileBasename"], "close");
        assert_eq!(job["onFinish"][0]["actionName"], "markAsExported");
        assert_eq!(job["onFinish"][0]["label"], "acme-etl");
        // A string, per the doc's parameter table — not a JSON boolean.
        assert_eq!(job["test"], json!("true"));
        assert!(request.template_source().unwrap().contains("#list reports"));
        assert!(
            !job.contains_key("credentials"),
            "credentials added at send"
        );
    }

    #[test]
    fn export_defaults_to_csv_even_for_json_templates() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let template: crate::ExportTemplate<crate::Json<Vec<u8>>> =
            crate::ExportTemplate::typed("x");
        let action = client.export_reports(&template, ReportsQuery::report_ids(["R1"]));
        let request = export_reports(&action);
        assert_eq!(
            request.description()["outputSettings"]["fileExtension"],
            "csv"
        );
    }

    #[test]
    fn on_finish_email_and_sftp_shapes() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let template = crate::ExportTemplate::new("x");
        let action = client
            .export_reports(&template, ReportsQuery::report_ids(["R1"]))
            .on_finish(OnFinish::email("a@x.com,b@x.com").message("month end"))
            .on_finish(OnFinish::sftp_upload(SftpConnection {
                host: "sftp.acme.com".into(),
                login: "acme".into(),
                password: "hunter2-super-secret".into(),
                port: 22,
            }));
        let request = export_reports(&action);
        let job = request.wire_job();

        assert_eq!(job["onFinish"][0]["actionName"], "email");
        // Comma-separated string, not an array.
        assert_eq!(job["onFinish"][0]["recipients"], "a@x.com,b@x.com");
        assert_eq!(job["onFinish"][0]["message"], "month end");
        assert_eq!(job["onFinish"][1]["actionName"], "sftpUpload");
        assert_eq!(job["onFinish"][1]["sftpData"]["host"], "sftp.acme.com");
        assert_eq!(
            job["onFinish"][1]["sftpData"]["password"],
            "hunter2-super-secret"
        );
        assert_eq!(job["onFinish"][1]["sftpData"]["port"], json!(22));

        // The same field, as an observer sees it.
        let observed = request.observed_job();
        assert_eq!(observed["onFinish"][1]["sftpData"]["password"], REDACTED);
        assert_eq!(observed["onFinish"][1]["sftpData"]["host"], "sftp.acme.com");
    }

    #[test]
    fn feed_credentials_ride_inside_credentials() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.update_employees(EmployeeSource::FetchUrl {
            url: "https://hr.acme.com/feed.json".into(),
            user: Some("hr".into()),
            password: Some("hunter2-super-secret".into()),
        });
        let request = update_employees(&action);
        assert_eq!(request.description()["dataSource"], "download");
        // Not in inputSettings: the feed credentials nest in `credentials`.
        let extras = request.wire_credential_extras();
        assert_eq!(extras["feedUrl"], "https://hr.acme.com/feed.json");
        assert_eq!(extras["feedPassword"], "hunter2-super-secret");

        let action = client.update_employees(EmployeeSource::Sftp {
            connection: SftpConnection {
                host: "sftp.acme.com".into(),
                login: "acme".into(),
                password: "hunter2-super-secret".into(),
                port: 2222,
            },
            filename: "employees.json".into(),
        });
        let request = update_employees(&action);
        assert_eq!(request.description()["dataSource"], "sftp");
        let sftp = &request.credential_extras["sftp"];
        assert_eq!(sftp["host"], "sftp.acme.com");
        assert_eq!(sftp["filename"], "employees.json");
        assert_eq!(sftp["port"], json!(2222));
    }

    #[test]
    fn download_names_the_file_system() {
        let request = download("is_reconciliation_1.csv", FileSystem::Reconciliation);
        let job = request.description();
        assert_eq!(job["type"], "download");
        assert_eq!(job["fileName"], "is_reconciliation_1.csv");
        assert_eq!(job["fileSystem"], "reconciliation");
    }

    #[test]
    fn expense_uses_integer_cents_and_created() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.create_expenses(
            "ap@acme.com",
            [Expense::new(
                "Cloud Inc",
                date!(2026 - 07 - 31),
                Money::new(129_00, "USD"),
            )
            .category("Infrastructure")
            .external_id("hosting-2026-07")
            .tax(ExpenseTax::new("id_TAX_OPTION_16").amount_cents(600))],
        );
        let request = create_expenses(&action);
        // Required by Expensify, so it is never absent.
        assert_eq!(request.input()["employeeEmail"], "ap@acme.com");
        let txn = &request.input()["transactionList"][0];
        assert_eq!(txn["amount"], json!(12900));
        assert_eq!(txn["currency"], "USD");
        assert_eq!(txn["created"], "2026-07-31");
        assert_eq!(txn["externalID"], "hosting-2026-07");
        assert_eq!(txn["tax"]["rateID"], "id_TAX_OPTION_16");
        assert_eq!(txn["tax"]["amount"], json!(600));
        assert!(txn.get("comment").is_none(), "unset options are absent");
    }

    #[test]
    fn report_creator_normalizes_field_keys_and_uses_date() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client
            .create_report(
                "P1",
                "user@example.com",
                "July",
                [ExpenseLine::new(
                    "Taxi",
                    date!(2026 - 07 - 04),
                    Money::new(2_50, "USD"),
                )],
            )
            .report_field("Reason of trip!", "Business trip");
        let request = create_report(&action);
        let input = request.input();
        assert_eq!(
            input["report"]["fields"]["Reason_of_trip_"],
            "Business trip"
        );
        assert_eq!(input["expenses"][0]["date"], "2026-07-04");
        assert_eq!(input["expenses"][0]["amount"], json!(250));
    }

    #[test]
    fn tag_approver_clear_sends_empty_string() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.set_tag_approvers(
            "P1",
            [
                TagApprover::assign("Engineering", "cto@example.com"),
                TagApprover::clear("Legal"),
            ],
        );
        let job = create_tag_approvers(&action);
        assert_eq!(job["tagApprovers"][0]["approver"], "cto@example.com");
        assert_eq!(job["tagApprovers"][1]["approver"], "");
    }

    fn create_tag_approvers(action: &SetTagApproversAction) -> Map<String, Value> {
        set_tag_approvers(action).description().clone()
    }

    #[test]
    fn tag_csv_sends_csv_not_cvs() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.update_policy("P1").tags(TagsUpdate::replace_all_csv(
            Bytes::from_static(b"Dept,Team\nEng,Core\n"),
            TagCsvConfig::dependent(true)
                .with_header_row()
                .with_gl_codes(),
        ));
        let request = update_policy(&action);
        let job = request.description();
        assert_eq!(job["tags"]["action"], "replace");
        assert_eq!(job["tags"]["source"], "file");
        assert_eq!(job["tags"]["config"]["fileType"], "csv");
        assert_eq!(job["tags"]["config"]["dependency"], json!(true));
        // Dependent levels take a scalar setRequired.
        assert_eq!(job["tags"]["config"]["setRequired"], json!(true));
        assert_eq!(job["tags"]["config"]["header"], json!(true));
        assert_eq!(job["tags"]["config"]["glCodes"], json!(true));
        assert!(request.file_field().unwrap().starts_with("Dept,Team"));
    }

    #[test]
    fn independent_tag_csv_sends_per_level_set_required() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.update_policy("P1").tags(TagsUpdate::replace_all_csv(
            Bytes::from_static(b"a,b\n"),
            TagCsvConfig::independent([true, false]).tsv(),
        ));
        let job = update_policy(&action).description().clone();
        assert_eq!(job["tags"]["config"]["setRequired"], json!([true, false]));
        assert_eq!(job["tags"]["config"]["fileType"], "tsv");
        assert_eq!(job["tags"]["config"]["dependency"], json!(false));
    }

    #[test]
    fn policy_update_camel_cases_and_uses_cents() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client
            .update_policy("P1")
            .categories(crate::CategoriesUpdate::merge([Category::new("Meals")
                .gl_code("6000")
                .payroll_code("MEAL")
                .comment_hint("why?")
                .require_comments()
                .max_expense_amount_cents(50_00)]))
            .report_fields(crate::ReportFieldsUpdate::replace_all([
                ReportFieldDef::new("Cost Center", ReportFieldDefType::Dropdown)
                    .values([ReportFieldValue::new("Ops").external_id("X1")])
                    .default_value("Ops"),
            ]))
            .tags(TagsUpdate::replace_all_inline([TagLevel::new([
                PolicyTag::new("Core").gl_code("7000"),
            ])
            .named("Team")
            .required()]));
        let job = update_policy(&action).description().clone();

        assert_eq!(job["inputSettings"]["policyID"], "P1");
        let category = &job["categories"]["data"][0];
        assert_eq!(job["categories"]["action"], "merge");
        assert_eq!(category["glCode"], "6000");
        assert_eq!(category["payrollCode"], "MEAL");
        assert_eq!(category["commentHint"], "why?");
        assert_eq!(category["areCommentsRequired"], json!(true));
        assert_eq!(category["maxExpenseAmount"], json!(5000));

        let field = &job["reportFields"]["data"][0];
        assert_eq!(job["reportFields"]["action"], "replace");
        assert_eq!(field["type"], "dropdown");
        assert_eq!(field["defaultValue"], "Ops");
        assert_eq!(field["values"][0]["externalID"], "X1");
        assert_eq!(field["values"][0]["enabled"], json!(true));

        let level = &job["tags"]["data"][0];
        assert_eq!(job["tags"]["source"], "inline");
        assert_eq!(level["name"], "Team");
        assert_eq!(level["setRequired"], json!(true));
        assert_eq!(level["tags"][0]["glCode"], "7000");
    }

    #[test]
    fn update_policies_uses_the_list_key() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client.update_policies(["P1", "P2"]);
        let job = update_policy(&action).description().clone();
        assert_eq!(job["inputSettings"]["policyIDList"], json!(["P1", "P2"]));
        assert!(job["inputSettings"].get("policyID").is_none());
    }

    #[test]
    fn expense_rule_creator_nests_the_actions() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client
            .create_expense_rule("P1", "user@example.com")
            .tag("Core")
            .default_billable(true);
        let input = create_expense_rule(&action).input().clone();

        assert_eq!(input["type"], "expenseRules");
        assert_eq!(input["policyID"], "P1");
        assert_eq!(input["employeeEmail"], "user@example.com");
        assert_eq!(input["actions"]["tag"], "Core");
        assert_eq!(input["actions"]["defaultBillable"], json!(true));
        assert!(input.get("ruleID").is_none(), "creator has no rule to name");
    }

    #[test]
    fn expense_rule_updater_sends_the_rule_id_as_an_integer() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client
            .update_expense_rule("P1", "user@example.com", crate::RuleId(4242))
            .tag("Core");
        let request = update_expense_rule(&action);
        let input = request.input();

        assert_eq!(request.description()["type"], "update");
        assert_eq!(input["type"], "expenseRules");
        assert_eq!(input["ruleID"], json!(4242));
        assert_eq!(input["actions"]["tag"], "Core");
        // Unset knobs are absent, not null.
        assert!(input["actions"].get("defaultBillable").is_none());
    }

    #[cfg(feature = "employee-updater-deprecated")]
    #[test]
    fn deprecated_csv_updater_declares_its_file_type() {
        let request = update_employees_csv(&PolicyId::new("P1"), Bytes::from_static(b"a,b\n"));
        let input = request.input();
        assert_eq!(input["type"], "employees");
        assert_eq!(input["policyID"], "P1");
        assert_eq!(input["fileType"], "csv");
    }

    #[test]
    fn reimburse_pins_the_status() {
        let request = reimburse(&ReimburseTargets::report_ids(["R1", "R2"]), Some("ACME-AP"));
        let input = request.input();
        assert_eq!(input["status"], "REIMBURSED");
        assert_eq!(input["paymentSource"], "ACME-AP");
        assert_eq!(input["filters"]["reportIDList"], "R1,R2");
    }

    #[test]
    fn employee_feed_rides_in_the_data_field() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let action = client
            .update_employees(EmployeeSource::Inline(vec![
                Employee::new("e@x.com", "m@x.com", "42", "P1")
                    .approval_limit(500_00)
                    .role(crate::PolicyRole::Admin)
                    .terminated(),
            ]))
            .dry_run();
        let request = update_employees(&action);
        let job = request.description();
        assert_eq!(job["dataSource"], "request");
        assert_eq!(job["dry-run"], json!(true));
        assert_eq!(job["inputSettings"]["entity"], "generic");

        let feed: Value = serde_json::from_str(request.data_field().unwrap()).unwrap();
        assert_eq!(feed[0]["employeeID"], "42");
        assert_eq!(feed[0]["policyID"], "P1");
        assert_eq!(feed[0]["approvalLimit"], json!(50000));
        assert_eq!(feed[0]["role"], "admin");
        assert_eq!(feed[0]["isTerminated"], json!(true));
    }

    #[test]
    fn reconciliation_pins_synchronous_mode() {
        let client = crate::Client::new(crate::Credentials::new("id", "secret"));
        let template = crate::ReconciliationTemplate::new("<#list cards as c, r></#list>");
        let action = client.domain("acme.com").reconcile(
            &template,
            date!(2026 - 07 - 01),
            date!(2026 - 07 - 31),
            ReconciliationScope::All,
        );
        let request = reconcile(&action);
        let input = request.input();
        assert_eq!(input["domain"], "acme.com");
        assert_eq!(input["type"], "All");
        assert_eq!(input["async"], json!(false));
        assert_eq!(input["feed"], "export_all_feeds");
        assert_eq!(request.description()["type"], "reconciliation");
    }

    #[test]
    fn empty_tax_object_becomes_null() {
        assert_eq!(normalize_tax(json!({})), Value::Null);
        assert_eq!(normalize_tax(json!({"name": "VAT"}))["name"], "VAT");
    }

    #[test]
    fn report_field_keys_replace_non_alphanumerics() {
        assert_eq!(
            normalize_report_field_key("Reason of trip"),
            "Reason_of_trip"
        );
        assert_eq!(
            normalize_report_field_key("cost-center #1"),
            "cost_center__1"
        );
    }
}
