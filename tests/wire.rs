//! Wire-level behaviour against a mock Integration Server.
//!
//! These exercise the paths that only exist because Expensify's envelope is
//! decoupled from HTTP semantics — most of all that a 200 can carry a
//! failure, and that a download body is content rather than an envelope.

use expensify::{
    ApiErrorKind, Client, Credentials, Error, ExportFormat, ExportTemplate, FileSystem, Json,
    PolicyField, PolicyPlan, PolicyRole, PolicyTags, ReconciliationScope, ReconciliationTemplate,
    ReimburseTargets, ReportFieldType, ReportsQuery, Url,
};
use serde::Deserialize;
use serde_json::{Value, json};
use time::macros::date;
use wiremock::matchers::method;
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

/// Every job posts to the same path, so mocks are told apart by the `type`
/// inside the urlencoded `requestJobDescription` field.
struct JobType(&'static str);

impl Match for JobType {
    fn matches(&self, request: &Request) -> bool {
        job(request).is_some_and(|job| job["type"] == self.0)
    }
}

fn form(request: &Request) -> Vec<(String, String)> {
    serde_urlencoded::from_bytes(&request.body).expect("body is form-urlencoded")
}

fn field(request: &Request, name: &str) -> Option<String> {
    form(request)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn job(request: &Request) -> Option<Value> {
    serde_json::from_str(&field(request, "requestJobDescription")?).ok()
}

async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer) -> Client {
    Client::builder(Credentials::new("partner-id", "partner-secret"))
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .build()
}

// ---------------------------------------------------------------------------
// HTTP 200 does not imply success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn body_response_code_beats_http_200() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 410,
        "responseMessage": "Required parameter 'policyName' is missing",
    })))
    .await;

    let err = client(&server)
        .create_policy("Ops")
        .await
        .expect_err("HTTP 200 with responseCode 410 must fail");

    match err {
        Error::Api(api) => {
            assert_eq!(api.kind, ApiErrorKind::Validation);
            assert_eq!(api.code, 410);
            assert!(api.message.unwrap().contains("policyName"));
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn body_403_maps_to_invalid_permissions() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 403,
        "responseMessage": "You are not an admin on this policy",
    })))
    .await;

    let err = client(&server).list_policies().await.unwrap_err();
    match err {
        Error::Api(api) => assert_eq!(api.kind, ApiErrorKind::InvalidPermissions),
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn http_200_with_response_code_200_succeeds() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyID": "0123456789ABCDEF",
        "policyName": "My New Policy",
    })))
    .await;

    let created = client(&server)
        .create_policy("My New Policy")
        .await
        .unwrap();
    assert_eq!(created.policy_id.as_str(), "0123456789ABCDEF");
    assert_eq!(created.name, "My New Policy");
}

// ---------------------------------------------------------------------------
// 429 from both layers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_status_429_is_rate_limited() {
    let server = server_with(
        ResponseTemplate::new(429)
            .insert_header("retry-after", "17")
            .set_body_string("Too Many Requests"),
    )
    .await;

    match client(&server).list_policies().await.unwrap_err() {
        Error::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(std::time::Duration::from_secs(17)));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn body_code_429_is_rate_limited() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 429,
        "responseMessage": "Too many requests",
    })))
    .await;

    match client(&server).list_policies().await.unwrap_err() {
        Error::RateLimited { retry_after: None } => {}
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// export -> download, and the fileSystem that must reach the wire
// ---------------------------------------------------------------------------

/// The envelope form of the exporter's submit response. Expensify has only
/// ever been seen answering a bare filename (`tests/replay.rs`); this shape is
/// the documented one and is still accepted, which is why the two decisions —
/// what the file system is, and how the name arrived — stay independent.
#[tokio::test]
async fn export_then_download_sends_integration_server() {
    let server = MockServer::start().await;
    Mock::given(JobType("file"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "responseCode": 200,
            "responseMessage": "OK",
            "filename": "export_8675309.csv",
        })))
        .mount(&server)
        .await;
    Mock::given(JobType("download"))
        .respond_with(ResponseTemplate::new(200).set_body_string("merchant,amount\nTaxi,250\n"))
        .mount(&server)
        .await;

    let client = client(&server);
    let template = ExportTemplate::new("<#list reports as report></#list>");

    let file = client
        .export_reports(&template, ReportsQuery::since(date!(2026 - 07 - 01)))
        .await
        .unwrap();
    assert_eq!(file.name(), "export_8675309.csv");
    assert_eq!(file.file_system(), FileSystem::IntegrationServer);

    let bytes = client.download(&file).await.unwrap();
    assert_eq!(&bytes[..], b"merchant,amount\nTaxi,250\n");

    let requests = server.received_requests().await.unwrap();
    let export = job(&requests[0]).unwrap();
    assert_eq!(
        export["onReceive"]["immediateResponse"][0],
        "returnRandomFileName"
    );
    assert!(
        field(&requests[0], "template")
            .unwrap()
            .contains("#list reports")
    );
    // Credentials are injected by the client, never by the caller.
    assert_eq!(export["credentials"]["partnerUserID"], "partner-id");

    let download = job(&requests[1]).unwrap();
    assert_eq!(download["fileName"], "export_8675309.csv");
    assert_eq!(download["fileSystem"], "integrationServer");
}

#[tokio::test]
async fn reconcile_then_download_sends_reconciliation() {
    let server = MockServer::start().await;
    Mock::given(JobType("reconciliation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "filename": "is_reconciliation_5429137734434770049.csv",
            "responseMessage": "OK",
            "responseCode": 200,
        })))
        .mount(&server)
        .await;
    Mock::given(JobType("download"))
        .respond_with(ResponseTemplate::new(200).set_body_string("card,total\n1979,900\n"))
        .mount(&server)
        .await;

    let client = client(&server);
    let template = ReconciliationTemplate::new("<#list cards as card, reports></#list>");

    let file = client
        .domain("acme.com")
        .reconcile(
            &template,
            date!(2026 - 07 - 01),
            date!(2026 - 07 - 31),
            ReconciliationScope::Unreported,
        )
        .await
        .unwrap();
    assert_eq!(file.file_system(), FileSystem::Reconciliation);

    client.download(&file).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let download = job(&requests[1]).unwrap();
    // The whole point of the phantom chain: the producer picked this, and the
    // caller had no way to pick the other one.
    assert_eq!(download["fileSystem"], "reconciliation");
    assert_eq!(
        download["fileName"],
        "is_reconciliation_5429137734434770049.csv"
    );
}

#[tokio::test]
async fn download_error_envelope_is_not_treated_as_content() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 404,
        "responseMessage": "File not found",
    })))
    .await;

    let file = expensify::ExportedFile::from_parts("missing.csv", FileSystem::IntegrationServer);
    match client(&server).download(&file).await.unwrap_err() {
        Error::Api(api) => assert_eq!(api.kind, ApiErrorKind::NotFound),
        other => panic!("expected Api, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// typed templates
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, PartialEq)]
struct ReportRow {
    report_id: String,
    total_cents: i64,
}

#[tokio::test]
async fn json_template_round_trips_to_typed_rows() {
    let server = MockServer::start().await;
    Mock::given(JobType("file"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "responseCode": 200,
            "filename": "close_1.json",
        })))
        .mount(&server)
        .await;
    Mock::given(JobType("download"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[{"report_id":"R1","total_cents":12900},{"report_id":"R2","total_cents":4250}]"#,
        ))
        .mount(&server)
        .await;

    let client = client(&server);
    let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed("...");

    let file = client
        .export_reports(&template, ReportsQuery::report_ids(["R1", "R2"]))
        .format(ExportFormat::Json)
        .await
        .unwrap();

    // No turbofish, no unwrap of an Option: the handle carries the type.
    let rows = client.download(&file).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ReportRow {
                report_id: "R1".into(),
                total_cents: 12_900
            },
            ReportRow {
                report_id: "R2".into(),
                total_cents: 4_250
            },
        ]
    );
}

#[tokio::test]
async fn json_template_decode_failure_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(JobType("file"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"responseCode": 200, "filename": "close_1.csv"})),
        )
        .mount(&server)
        .await;
    Mock::given(JobType("download"))
        // What a Json<_> template gets when the format defaulted to csv.
        .respond_with(ResponseTemplate::new(200).set_body_string("report_id,total\nR1,129\n"))
        .mount(&server)
        .await;

    let client = client(&server);
    let template: ExportTemplate<Json<Vec<ReportRow>>> = ExportTemplate::typed("...");
    let file = client
        .export_reports(&template, ReportsQuery::report_ids(["R1"]))
        .await
        .unwrap();

    assert!(matches!(
        client.download(&file).await.unwrap_err(),
        Error::Decode(_)
    ));
}

// ---------------------------------------------------------------------------
// 207 partial reimbursement
// ---------------------------------------------------------------------------

fn partial_207() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 207,
        "reportIDs": ["R00bCluvcO4T"],
        "skippedReports": [
            { "reason": "Report is in status 'Open'", "reportID": "R006AseGxMka" }
        ],
        "failedReports": [
            { "reason": "Internal error", "reportID": "R002bGmt16ac" }
        ],
    }))
}

#[tokio::test]
async fn strict_reimbursement_rejects_207() {
    let server = server_with(partial_207()).await;

    let err = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::report_ids([
            "R00bCluvcO4T",
            "R006AseGxMka",
            "R002bGmt16ac",
        ]))
        .payment_source("ACME-AP")
        .await
        .expect_err("207 must not look like success");

    match err {
        Error::PartialSuccess(outcome) => {
            assert_eq!(outcome.updated.len(), 1);
            assert_eq!(outcome.skipped[0].report_id.as_str(), "R006AseGxMka");
            assert!(outcome.skipped[0].reason.contains("Open"));
            assert_eq!(outcome.failed[0].report_id.as_str(), "R002bGmt16ac");
        }
        other => panic!("expected PartialSuccess, got {other:?}"),
    }
}

#[tokio::test]
async fn tolerant_reimbursement_accepts_207() {
    let server = server_with(partial_207()).await;

    let outcome = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::report_ids(["R00bCluvcO4T"]))
        .tolerate_partial()
        .await
        .expect("tolerate_partial makes 207 an Ok");

    assert_eq!(outcome.updated[0].as_str(), "R00bCluvcO4T");
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.failed.len(), 1);

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["type"], "reportStatus");
    assert_eq!(input["status"], "REIMBURSED");
}

#[tokio::test]
async fn strict_reimbursement_accepts_200() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "reportIDs": ["R1", "R2"],
    })))
    .await;

    let updated = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::since(date!(2026 - 07 - 01)))
        .await
        .unwrap();
    assert_eq!(updated.len(), 2);
}

// ---------------------------------------------------------------------------
// policy getter typestate against a real response body
// ---------------------------------------------------------------------------

#[tokio::test]
async fn policy_getter_populates_only_requested_sections() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": {
            "0123456789ABCDEF": {
                "categories": [
                    { "name": "Infrastructure", "enabled": true, "glCode": "6000",
                      "areCommentsRequired": false, "maxExpenseAmount": 50000 }
                ],
                "tax": {
                    "name": "VAT",
                    "default": "id_TAX_OPTION_16",
                    "rates": [
                        { "name": "Standard", "rate": 20.0, "rateID": "id_TAX_OPTION_16" }
                    ]
                }
            }
        }
    })))
    .await;

    let id = expensify::PolicyId::new("0123456789ABCDEF");
    let policies = client(&server)
        .get_policies([&id])
        .with_categories()
        .with_tax()
        .await
        .unwrap();

    let policy = &policies[&id];
    assert_eq!(policy.categories[0].name, "Infrastructure");
    assert_eq!(policy.categories[0].gl_code.as_deref(), Some("6000"));
    assert_eq!(policy.categories[0].max_expense_amount_cents, Some(50_000));
    let tax = policy.tax.as_ref().expect("policy has tax configured");
    assert_eq!(tax.rates[0].rate_id.as_str(), "id_TAX_OPTION_16");

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["fields"], json!(["categories", "tax"]));
    assert_eq!(input["policyIDList"], json!(["0123456789ABCDEF"]));
}

#[tokio::test]
async fn empty_tax_object_is_none_not_an_error() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": { "P1": { "tax": {} } }
    })))
    .await;

    let id = expensify::PolicyId::new("P1");
    let policies = client(&server)
        .get_policies([&id])
        .with_tax()
        .await
        .unwrap();
    assert!(policies[&id].tax.is_none());
}

/// Expensify's documented sample returns `tags` flat for one policy and
/// level-wrapped for another. Before both shapes were modelled, the second
/// one failed the whole call and discarded the first policy's data.
#[tokio::test]
async fn policy_getter_accepts_both_tag_shapes_in_one_call() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": {
            "FLAT": { "tags": [{ "glCode": "", "name": "Enterprise", "enabled": true }] },
            "LEVELS": { "tags": [{ "name": "Tags", "tags": [] }] }
        }
    })))
    .await;

    let flat = expensify::PolicyId::new("FLAT");
    let levels = expensify::PolicyId::new("LEVELS");
    let policies = client(&server)
        .get_policies([&flat, &levels])
        .with_tags()
        .await
        .expect("a level-wrapped policy must not poison the whole call");

    match &policies[&flat].tags {
        PolicyTags::Flat(tags) => assert_eq!(tags[0].name, "Enterprise"),
        other => panic!("expected a flat list, got {other:?}"),
    }
    match &policies[&levels].tags {
        PolicyTags::Levels(levels) => {
            assert_eq!(levels[0].name.as_deref(), Some("Tags"));
            assert!(levels[0].tags.is_empty());
        }
        other => panic!("expected levels, got {other:?}"),
    }
}

#[tokio::test]
async fn policy_getter_reads_report_fields_and_employees() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": {
            "P1": {
                "reportFields": [
                    { "name": "Title", "type": "formula", "values": [] },
                    { "name": "Cost Center", "type": "dropdown", "values": ["Ops", "Eng"] }
                ],
                "employees": [
                    { "email": "user@acme.com", "role": "admin",
                      "submitsTo": "boss@acme.com", "employeeID": "42" }
                ]
            }
        }
    })))
    .await;

    let id = expensify::PolicyId::new("P1");
    let policies = client(&server)
        .get_policies([&id])
        .with_report_fields()
        .with_employees()
        .await
        .unwrap();

    let policy = &policies[&id];
    assert_eq!(policy.report_fields[0].field_type, ReportFieldType::Formula);
    assert_eq!(policy.report_fields[1].values, ["Ops", "Eng"]);
    assert_eq!(policy.employees[0].role, PolicyRole::Admin);
    assert_eq!(policy.employees[0].employee_id.as_deref(), Some("42"));

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["fields"], json!(["reportFields", "employees"]));
}

// ---------------------------------------------------------------------------
// policy getter, dynamic escape hatch
// ---------------------------------------------------------------------------

fn two_section_policy() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": {
            "P1": {
                "categories": [{ "name": "Infrastructure", "enabled": true }],
                "tax": { "name": "VAT", "default": "id_A",
                         "rates": [{ "name": "Standard", "rate": 20.0, "rateID": "id_A" }] }
            }
        }
    }))
}

#[tokio::test]
async fn dynamic_getter_populates_only_requested_sections() {
    let server = server_with(two_section_policy()).await;

    let id = expensify::PolicyId::new("P1");
    let policies = client(&server)
        .get_policies_dynamic([&id], [PolicyField::Categories, PolicyField::Tax])
        .await
        .unwrap();

    let policy = &policies[&id];
    assert_eq!(
        policy.categories.as_ref().unwrap()[0].name,
        "Infrastructure"
    );
    assert_eq!(policy.tax.as_ref().unwrap().as_ref().unwrap().name, "VAT");
    // Not requested: absent, not empty.
    assert!(policy.report_fields.is_none());
    assert!(policy.tags.is_none());
    assert!(policy.employees.is_none());

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["fields"], json!(["categories", "tax"]));
}

/// The point of sharing one request path: a selection made either way must
/// put the same bytes on the wire.
#[tokio::test]
async fn dynamic_and_static_selections_send_the_same_request() {
    let server = server_with(two_section_policy()).await;
    let id = expensify::PolicyId::new("P1");

    client(&server)
        .get_policies([&id])
        .with_categories()
        .with_tax()
        .await
        .unwrap();
    client(&server)
        .get_policies_dynamic([&id], &[PolicyField::Categories, PolicyField::Tax])
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        job(&requests[0]).unwrap()["inputSettings"],
        job(&requests[1]).unwrap()["inputSettings"]
    );
}

/// The static path cannot select a field twice; a `Vec` can.
#[tokio::test]
async fn a_repeated_field_is_sent_once() {
    let server = server_with(two_section_policy()).await;

    client(&server)
        .get_policies_dynamic(["P1"], [PolicyField::Tax, PolicyField::Tax])
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["fields"], json!(["tax"]));
}

/// The compile-time "at least one field" rule (`GetPoliciesBuilder` is not a
/// future) has no compile-time equivalent here, so it becomes a pre-flight
/// rejection — and, as with the empty-ID list, nothing is sent.
#[tokio::test]
async fn an_empty_dynamic_selection_is_rejected_before_sending() {
    let server = server_with(two_section_policy()).await;

    let err = client(&server)
        .get_policies_dynamic(["P1"], Vec::<PolicyField>::new())
        .await
        .expect_err("an empty fields list is a documented 410");

    match err {
        Error::InvalidRequest(message) => {
            assert!(message.contains("at least one field"), "{message}")
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// The escape hatch gives up compile-time shaping, not the decode checks:
/// a section that was asked for and did not come back is still an error, not
/// a silent `None`.
#[tokio::test]
async fn a_requested_section_missing_from_the_response_is_an_error() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": { "P1": { "categories": [] } }
    })))
    .await;

    let err = client(&server)
        .get_policies_dynamic(["P1"], [PolicyField::Categories, PolicyField::Employees])
        .await
        .expect_err("`employees` was requested and is absent");

    match err {
        Error::Decode(decode) => assert!(format!("{decode}").contains("employees"), "{decode}"),
        other => panic!("expected Decode, got {other:?}"),
    }
}

/// `on_behalf_of` is the one setter both getters carry.
#[tokio::test]
async fn dynamic_getter_carries_on_behalf_of() {
    let server = server_with(two_section_policy()).await;

    client(&server)
        .get_policies_dynamic(["P1"], [PolicyField::Tax])
        .on_behalf_of("finance@acme.com")
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["userEmail"], "finance@acme.com");
}

/// A statically-typed result projects down to the same shape the dynamic
/// getter returns.
#[tokio::test]
async fn a_static_result_projects_to_the_dynamic_shape() {
    let server = server_with(two_section_policy()).await;
    let id = expensify::PolicyId::new("P1");

    let statically = client(&server)
        .get_policies([&id])
        .with_categories()
        .with_tax()
        .await
        .unwrap();
    let dynamically = client(&server)
        .get_policies_dynamic([&id], [PolicyField::Categories, PolicyField::Tax])
        .await
        .unwrap();

    let projected = statically[&id].clone().project();
    assert_eq!(projected.categories, dynamically[&id].categories);
    assert_eq!(projected.tax, dynamically[&id].tax);
    assert!(projected.employees.is_none());
}

/// `PolicySummary`'s renames (`type` -> plan, `outputCurrency`) are only
/// exercised here, and one policy on an unmodelled plan must not fail the
/// list.
#[tokio::test]
async fn policy_list_decodes_summaries_including_unknown_plans() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyList": [
            { "id": "P1", "name": "Ops", "owner": "boss@acme.com",
              "role": "admin", "outputCurrency": "USD", "type": "corporate" },
            { "id": "P2", "name": "Personal", "owner": "me@acme.com",
              "role": "user", "outputCurrency": "GBP", "type": "personalPolicy" }
        ]
    })))
    .await;

    let policies = client(&server).list_policies().admin_only().await.unwrap();
    assert_eq!(policies[0].id.as_str(), "P1");
    assert_eq!(policies[0].plan, PolicyPlan::Corporate);
    assert_eq!(policies[0].output_currency.as_str(), "USD");
    assert_eq!(policies[1].plan, PolicyPlan::Other("personalPolicy".into()));
    assert_eq!(policies[1].role, PolicyRole::User);

    let requests = server.received_requests().await.unwrap();
    let input = &job(&requests[0]).unwrap()["inputSettings"];
    assert_eq!(input["type"], "policyList");
    assert_eq!(input["adminOnly"], json!(true));
}

#[tokio::test]
async fn missing_requested_section_is_a_decode_error() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyInfo": { "P1": {} }
    })))
    .await;

    let err = client(&server)
        .get_policies(["P1"])
        .with_categories()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Decode(_)), "{err:?}");
}

// ---------------------------------------------------------------------------
// remaining jobs, end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expense_creator_parses_the_transaction_list() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "transactionList": [
            { "amount": 12900, "merchant": "Cloud Hosting Inc", "created": "2026-07-31",
              "transactionID": "T123", "reportID": "R1", "currency": "USD" }
        ]
    })))
    .await;

    let created = client(&server)
        .create_expenses(
            "ap@acme.com",
            [expensify::Expense::new(
                "Cloud Hosting Inc",
                date!(2026 - 07 - 31),
                expensify::Money::new(12_900, "USD"),
            )],
        )
        .await
        .unwrap();

    assert_eq!(created[0].transaction_id.as_str(), "T123");
    assert_eq!(created[0].amount_cents, 12_900);
    assert_eq!(created[0].created, date!(2026 - 07 - 31));
}

#[tokio::test]
async fn domain_card_list_blanks_become_none() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "domainCardList": [
            { "bank": "Amex", "cardID": 4242, "cardName": "Ops card",
              "cardNumber": "1234XXXXXXXX1979", "email": "ops@acme.com",
              "externalEmployeeID": "", "created": "2026-01-02 03:04:05",
              "lastImport": "", "lastImportResult": 200,
              "reimbursable": false, "scrapeMinDate": "" }
        ]
    })))
    .await;

    let cards = client(&server)
        .domain("acme.com")
        .card_list()
        .await
        .unwrap();
    let card = &cards[0];
    assert_eq!(card.card_id, 4242);
    assert!(card.created.is_some());
    assert!(card.last_import.is_none());
    assert!(card.scrape_min_date.is_none());
    assert!(card.external_employee_id.is_none());
}

#[tokio::test]
async fn employee_updater_parses_the_diff() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "dry-run": true,
        "updatedEmployeesCount": 3,
        "diff": {
            "diffToAdd": { "P1": ["new@acme.com"] },
            "diffToRemove": { "P1": ["gone@acme.com"] }
        },
        "securityGroupEmployeesMap": { "G1": ["new@acme.com"] },
        "skippedEmployees": [{ "email": "bad@acme.com", "reason": "no manager" }]
    })))
    .await;

    let outcome = client(&server)
        .update_employees(expensify::EmployeeSource::Inline(vec![
            expensify::Employee::new("new@acme.com", "boss@acme.com", "42", "P1"),
        ]))
        .dry_run()
        .await
        .unwrap();

    assert!(outcome.dry_run);
    assert_eq!(outcome.updated_count, 3);
    assert_eq!(
        outcome.added[&expensify::PolicyId::new("P1")][0],
        "new@acme.com"
    );
    assert_eq!(outcome.removed[&expensify::PolicyId::new("P1")].len(), 1);
    assert_eq!(outcome.security_group_assignments["G1"].len(), 1);
    assert_eq!(outcome.skipped[0].reason, "no manager");
}

#[tokio::test]
async fn report_fields_that_are_not_an_object_fail_at_await() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "reportID": "R1",
        "reportName": "July",
    })))
    .await;

    let err = client(&server)
        .create_report("P1", "user@acme.com", "July", [])
        .report_fields(&vec!["not", "an", "object"])
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn report_creator_returns_the_new_report() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "reportID": "R00bCluvcO4T",
        "reportName": "July",
    })))
    .await;

    let created = client(&server)
        .create_report(
            "P1",
            "user@acme.com",
            "July",
            [expensify::ExpenseLine::new(
                "Taxi",
                date!(2026 - 07 - 04),
                expensify::Money::new(2_50, "USD"),
            )],
        )
        .report_field("Reason of trip!", "Business trip")
        .await
        .unwrap();

    assert_eq!(created.report_id.as_str(), "R00bCluvcO4T");
    assert_eq!(created.name, "July");

    let input = &job(&server.received_requests().await.unwrap()[0]).unwrap()["inputSettings"];
    assert_eq!(
        input["report"]["fields"]["Reason_of_trip_"],
        "Business trip"
    );
    assert_eq!(input["expenses"][0]["date"], "2026-07-04");
}

#[tokio::test]
async fn expense_rules_round_trip() {
    let server =
        server_with(ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200 }))).await;

    let client = client(&server);
    client
        .create_expense_rule("P1", "user@acme.com")
        .tag("Core")
        .await
        .unwrap();
    client
        .update_expense_rule("P1", "user@acme.com", expensify::RuleId(4242))
        .default_billable(false)
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let create = job(&requests[0]).unwrap();
    assert_eq!(create["type"], "create");
    assert_eq!(create["inputSettings"]["actions"]["tag"], "Core");

    let update = job(&requests[1]).unwrap();
    assert_eq!(update["type"], "update");
    assert_eq!(update["inputSettings"]["ruleID"], json!(4242));
    assert_eq!(
        update["inputSettings"]["actions"]["defaultBillable"],
        json!(false)
    );
}

#[tokio::test]
async fn tag_approvers_send_the_clear_sentinel() {
    let server =
        server_with(ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200 }))).await;

    client(&server)
        .set_tag_approvers(
            "P1",
            [
                expensify::TagApprover::assign("Engineering", "cto@acme.com"),
                expensify::TagApprover::clear("Legal"),
            ],
        )
        .await
        .unwrap();

    let sent = job(&server.received_requests().await.unwrap()[0]).unwrap();
    assert_eq!(sent["inputSettings"]["type"], "tagApprovers");
    assert_eq!(sent["tagApprovers"][0]["approver"], "cto@acme.com");
    assert_eq!(sent["tagApprovers"][1]["approver"], "");
}

// ---------------------------------------------------------------------------
// requests rejected before they are sent
// ---------------------------------------------------------------------------

/// An empty iterator type-checks through every anchored constructor and
/// produces exactly the filterless request Expensify answers 410 to. The
/// type system cannot see it; the client refuses to send it.
#[tokio::test]
async fn empty_collections_never_reach_the_wire() {
    let server =
        server_with(ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200 }))).await;
    let client = client(&server);
    let no_ids: [&str; 0] = [];

    let template = ExportTemplate::new("...");
    let errors = vec![
        client
            .export_reports(&template, ReportsQuery::report_ids(no_ids))
            .await
            .map(|_| ())
            .unwrap_err(),
        client
            .mark_reports_reimbursed(ReimburseTargets::report_ids(no_ids))
            .await
            .map(|_| ())
            .unwrap_err(),
        client
            .get_policies(no_ids)
            .with_tax()
            .await
            .map(|_| ())
            .unwrap_err(),
        client.update_policies(no_ids).await.unwrap_err(),
    ];

    for err in errors {
        assert!(matches!(err, Error::InvalidRequest(_)), "{err:?}");
    }
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing should have been sent"
    );
}

// ---------------------------------------------------------------------------
// download discrimination
// ---------------------------------------------------------------------------

/// An envelope with no `responseCode` is still an envelope. Handing this
/// back as file content writes Expensify's error text into the caller's
/// export.
#[tokio::test]
async fn download_envelope_without_a_code_is_not_content() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseMessage": "File not found",
    })))
    .await;

    let file = expensify::ExportedFile::from_parts("missing.csv", FileSystem::IntegrationServer);
    match client(&server).download(&file).await.unwrap_err() {
        Error::Decode(_) => {}
        other => panic!("expected Decode, got {other:?}"),
    }
}

/// The likeliest shape of the undocumented "export not rendered yet"
/// response. Reporting it as an empty success writes an empty export.
#[tokio::test]
async fn download_of_an_empty_body_is_an_error() {
    let server = server_with(ResponseTemplate::new(200).set_body_string("")).await;

    let file = expensify::ExportedFile::from_parts("pending.csv", FileSystem::IntegrationServer);
    match client(&server).download(&file).await.unwrap_err() {
        Error::Decode(_) => {}
        other => panic!("expected Decode, got {other:?}"),
    }
}

#[tokio::test]
async fn non_success_http_without_an_envelope_surfaces_the_status() {
    let server = server_with(
        ResponseTemplate::new(502).set_body_string("<html><body>Bad Gateway</body></html>"),
    )
    .await;

    match client(&server).list_policies().await.unwrap_err() {
        Error::Http { status, body } => {
            assert_eq!(status.as_u16(), 502);
            assert!(body.contains("Bad Gateway"));
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

#[cfg(feature = "employee-updater-deprecated")]
#[tokio::test]
#[allow(deprecated)]
async fn deprecated_csv_updater_uploads_multipart() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200, "nbEmployees": 42 })),
    )
    .await;

    let count = client(&server)
        .update_employees_csv("P1", bytes::Bytes::from_static(b"email,manager\n"))
        .await
        .unwrap();
    assert_eq!(count, 42);

    let request = &server.received_requests().await.unwrap()[0];
    let content_type = request.headers["content-type"].to_str().unwrap();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("requestJobDescription"));
    assert!(body.contains("email,manager"));
    assert!(body.contains(r#""fileType":"csv""#), "{body}");
}

/// DESIGN.md promises every action and output is `Send`, which is what
/// keeps them usable from a multi-threaded runtime. Phantoms are
/// `PhantomData<fn() -> T>` precisely so this holds for any `F`.
#[test]
fn actions_and_futures_are_send() {
    fn assert_send<T: Send>() {}

    assert_send::<expensify::ExportReportsAction<Json<Vec<ReportRow>>>>();
    assert_send::<expensify::DownloadAction<Json<Vec<ReportRow>>>>();
    assert_send::<expensify::ReconcileAction<Json<Vec<ReportRow>>>>();
    assert_send::<expensify::GetPoliciesAction<expensify::Fetched, expensify::Omitted>>();
    assert_send::<expensify::ReimburseAction>();
    assert_send::<expensify::ExportedFile<Json<Vec<ReportRow>>>>();
    assert_send::<expensify::UpdateEmployeesAction>();
}
