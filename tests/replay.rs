//! Live responses, recorded once and replayed forever.
//!
//! Every body in `tests/fixtures/` came off the real Integration Server on
//! 2026-08-04 through `ClientBuilder::observe`; nothing here is a shape this
//! crate inferred. That distinction is the point: five wire behaviours were
//! wrong in the first two releases, and every one of them was covered by a
//! mock that asserted the inference back at us.

use expensify::{
    Client, Credentials, Error, Expense, ExportTemplate, FileSystem, Money, ReimburseTargets,
    ReportsQuery, Url,
};
use serde_json::Value;
use time::macros::date;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// One recorded exchange: the body verbatim in a file, its status and
/// content-type beside it.
struct Fixture {
    body: &'static [u8],
    status: u16,
    /// `None` where the header was not recorded. It is never load-bearing:
    /// this endpoint answers JSON as `text/plain;charset=utf-8` for some jobs
    /// and as `application/json` for others, so nothing may key on it.
    content_type: Option<&'static str>,
}

/// Report Exporter, submit. A bare filename — no envelope, no JSON.
const EXPORT_SUBMIT: Fixture = Fixture {
    body: include_bytes!("fixtures/file-submit.txt"),
    status: 200,
    content_type: Some("text/plain;charset=utf-8"),
};

/// Report Status Updater over three Open reports: everything skipped, and
/// still `responseCode: 200`.
const REIMBURSE_ALL_SKIPPED: Fixture = Fixture {
    body: include_bytes!("fixtures/reimburse-all-skipped.json"),
    status: 200,
    content_type: Some("text/plain;charset=utf-8"),
};

/// The same job over one Approved and two Open reports. Also 200: Expensify
/// has not been observed answering 207 for either kind of partial run.
const REIMBURSE_MIXED: Fixture = Fixture {
    body: include_bytes!("fixtures/reimburse-mixed.json"),
    status: 200,
    content_type: Some("text/plain;charset=utf-8"),
};

const CREATE_EXPENSES: Fixture = Fixture {
    body: include_bytes!("fixtures/create-expenses.json"),
    status: 200,
    content_type: None,
};

/// Expense Rules Creator: an acknowledgement and nothing else — no rule ID,
/// which is why the action's output is `()`.
const CREATE_EXPENSE_RULES: Fixture = Fixture {
    body: include_bytes!("fixtures/create-expense-rules.json"),
    status: 200,
    content_type: Some("text/plain;charset=utf-8"),
};

async fn replaying(fixture: &Fixture) -> MockServer {
    let mut response = ResponseTemplate::new(fixture.status).set_body_bytes(fixture.body.to_vec());
    if let Some(content_type) = fixture.content_type {
        response = response.insert_header("content-type", content_type);
    }
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

fn job(request: &Request) -> Value {
    let form: Vec<(String, String)> =
        serde_urlencoded::from_bytes(&request.body).expect("body is form-urlencoded");
    let description = form
        .into_iter()
        .find(|(key, _)| key == "requestJobDescription")
        .map(|(_, value)| value)
        .expect("every job carries a description");
    serde_json::from_str(&description).expect("the description is JSON")
}

// ---------------------------------------------------------------------------
// defect 1: the exporter answers a bare filename
// ---------------------------------------------------------------------------

/// The crate's flagship operation, which never worked: the submit response is
/// the filename itself, so parsing it as the documented
/// `{"responseCode":200,"filename":…}` envelope failed with `expected value at
/// line 1 column 1`.
#[tokio::test]
async fn an_export_submit_answers_a_bare_filename() {
    let server = replaying(&EXPORT_SUBMIT).await;

    let template = ExportTemplate::new("<#list reports as report></#list>");
    let file = client(&server)
        .export_reports(&template, ReportsQuery::since(date!(2026 - 07 - 01)))
        .await
        .expect("a bare filename is the exporter's success shape");

    assert_eq!(
        file.name(),
        "export0fd99e06-a636-4974-b6bc-3ceb12163386.csv"
    );
    // The producer still pins the file system; nothing about that changed.
    assert_eq!(file.file_system(), FileSystem::IntegrationServer);
}

/// The bare body is accepted because of the *job*, not because of its
/// content-type — which lies. The same header on a JSON envelope still parses
/// as one, and a body-level error code still wins.
#[tokio::test]
async fn a_text_plain_error_envelope_is_still_an_error() {
    let server = replaying(&Fixture {
        body: br#"{"responseCode":410,"responseMessage":"Bad request. Verify the request parameters."}"#,
        status: 200,
        content_type: Some("text/plain;charset=utf-8"),
    })
    .await;

    let template = ExportTemplate::new("x");
    let err = client(&server)
        .export_reports(&template, ReportsQuery::report_ids(["R1"]))
        .await
        .expect_err("a 410 envelope is not a filename");
    match err {
        Error::Api(api) => assert_eq!(api.code, 410),
        other => panic!("expected Api, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// defect 3: partial reimbursement comes back 200
// ---------------------------------------------------------------------------

/// Three Open reports in, nothing reimbursed, `responseCode: 200`. Keying
/// strictness on 207 made this `Ok(vec![])` — success — and dropped all three
/// reasons.
#[tokio::test]
async fn a_reimbursement_that_skipped_everything_is_not_success() {
    let server = replaying(&REIMBURSE_ALL_SKIPPED).await;

    let err = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::report_ids([
            "R00Lza4SDrx4",
            "R00X9oNOn2MO",
            "R00PdJbQrzoC",
        ]))
        .await
        .expect_err("nothing moved; that is not success");

    match err {
        Error::PartialSuccess(outcome) => {
            assert!(outcome.updated.is_empty());
            assert_eq!(outcome.skipped.len(), 3);
            assert!(outcome.skipped[0].reason.contains("status 'Open'"));
            // `failedReports` is absent from the body, not `[]`.
            assert!(outcome.failed.is_empty());
        }
        other => panic!("expected PartialSuccess, got {other:?}"),
    }
}

/// The mixed case, which is the one that hides: one report moved, two did
/// not, still 200. Strict returning `Ok(["R00X9oNOn2MO"])` would look exactly
/// like a one-report run that worked.
#[tokio::test]
async fn a_mixed_reimbursement_is_not_success_either() {
    let server = replaying(&REIMBURSE_MIXED).await;

    let err = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::report_ids([
            "R00Lza4SDrx4",
            "R00X9oNOn2MO",
            "R00PdJbQrzoC",
        ]))
        .await
        .expect_err("two reports were skipped");

    match err {
        Error::PartialSuccess(outcome) => {
            assert_eq!(outcome.updated.len(), 1);
            assert_eq!(outcome.updated[0].as_str(), "R00X9oNOn2MO");
            assert_eq!(outcome.skipped.len(), 2);
            assert!(outcome.failed.is_empty());
        }
        other => panic!("expected PartialSuccess, got {other:?}"),
    }
}

/// The tolerant path is unchanged by any of this: the same body is data.
#[tokio::test]
async fn the_tolerant_path_reports_the_same_body_as_data() {
    let server = replaying(&REIMBURSE_MIXED).await;

    let outcome = client(&server)
        .mark_reports_reimbursed(ReimburseTargets::report_ids(["R00X9oNOn2MO"]))
        .tolerate_partial()
        .await
        .expect("tolerate_partial takes the outcome whatever the code");

    assert_eq!(outcome.updated.len(), 1);
    assert_eq!(outcome.skipped.len(), 2);
    assert!(outcome.failed.is_empty());
}

// ---------------------------------------------------------------------------
// defect 2: employeeEmail is required
// ---------------------------------------------------------------------------

/// Omitting `employeeEmail` is a 410 (`'employeeEmail' parameter is missing or
/// malformed`) with or without a policy, so it is an argument rather than a
/// setter and always reaches the wire.
#[tokio::test]
async fn the_expense_creator_always_names_the_employee() {
    let server = replaying(&CREATE_EXPENSES).await;

    let created = client(&server)
        .create_expenses(
            "user@example.com",
            [Expense::new(
                "Test Merchant",
                date!(2026 - 08 - 01),
                Money::new(1234, "USD"),
            )],
        )
        .await
        .expect("the recorded response is a success");

    let transaction = &created[0];
    assert_eq!(transaction.transaction_id.as_str(), "286636100957217088");
    assert_eq!(transaction.merchant, "Test Merchant");
    assert_eq!(transaction.created, date!(2026 - 08 - 01));
    assert_eq!(transaction.amount_cents, 1234);
    assert_eq!(transaction.currency.as_str(), "USD");

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(
        job(&requests[0])["inputSettings"]["employeeEmail"],
        "user@example.com"
    );
}

// ---------------------------------------------------------------------------
// defect 5's neighbour: the expense-rule response really is empty
// ---------------------------------------------------------------------------

/// `()` is the right output: the creator answers an acknowledgement with no
/// rule ID, which is why `update_expense_rule` needs one from elsewhere.
#[tokio::test]
async fn the_expense_rules_creator_answers_only_an_acknowledgement() {
    let server = replaying(&CREATE_EXPENSE_RULES).await;

    client(&server)
        .create_expense_rule("1234ABCD", "user@example.com")
        .tag("Core")
        .await
        .expect("OK is the whole response");
}
