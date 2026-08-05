//! Observability: what an observer sees, when, and what it is for.
//!
//! The motivating case has its own test
//! ([`a_plain_text_response_is_diagnosable_without_curl`]): an export that
//! answers a bare filename as `text/plain` instead of the JSON envelope this
//! crate expects, which surfaces to a caller as
//! `decode error: expected value at line 1 column 1` and says nothing about
//! why.

use std::sync::{Arc, Mutex};

use expensify::{
    Client, Credentials, Error, Exchange, ExportTemplate, ObservedRequest, Observer, Recorder,
    ReportsQuery, Url,
};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer, recorder: &Recorder) -> Client {
    Client::builder(Credentials::new("partner-id", "partner-secret"))
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .observe(recorder.clone())
        .build()
}

/// The incident this feature exists for, reproduced.
///
/// Expensify's Report Exporter answers the generated filename as bare
/// `text/plain`, not as the documented JSON envelope. The library's error says
/// only that column 1 was not JSON; the exchange says what column 1 actually
/// was. (Making the export *succeed* against this response is a separate fix;
/// what is asserted here is that the mismatch is legible without curl.)
#[tokio::test]
async fn a_plain_text_response_is_diagnosable_without_curl() {
    let server = server_with(
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/plain")
            .set_body_string("exportfilename_1754336489.csv"),
    )
    .await;

    let recorder = Recorder::new();
    let template = ExportTemplate::new("<#list reports as report></#list>");
    let err = client(&server, &recorder)
        .export_reports(&template, ReportsQuery::report_ids(["R006AseGxMka"]))
        .await
        .expect_err("a bare filename is not an envelope");

    // What the caller sees today, unchanged by this feature.
    assert!(
        matches!(err, Error::Decode(_)),
        "expected a decode error, got {err:?}"
    );
    // The sentence the incident was reported as: `error: decode error` /
    // `caused by: json: expected value at line 1 column 1`, and not one word
    // about what column 1 held.
    let chain = std::iter::successors(Some(&err as &dyn std::error::Error), |err| err.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    assert!(
        chain.contains("expected value at line 1 column 1"),
        "{chain}"
    );

    // What the observer adds: the three facts that identify the mismatch.
    let exchange = recorder.take().pop().expect("one exchange");
    assert_eq!(exchange.status(), 200);
    assert_eq!(exchange.content_type(), Some("text/plain"));
    assert_eq!(exchange.body_text(), "exportfilename_1754336489.csv");

    // ... and the request that provoked it, so the job type and filters are
    // not a second question.
    let request = exchange.request();
    assert_eq!(request.job_type(), "file");
    assert!(
        request.job_description().contains("R006AseGxMka"),
        "{request}"
    );

    // One rendering carries all of it, which is what the CLI prints.
    let rendered = exchange.to_string();
    for expected in [
        "text/plain",
        "exportfilename_1754336489.csv",
        "R006AseGxMka",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
}

/// The hook is on the client, not on an operation, so no job can be missed by
/// forgetting to opt it in.
#[tokio::test]
async fn every_job_is_observed() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyList": [],
        "policyInfo": {},
        "filename": "export_1.csv",
    })))
    .await;

    let recorder = Recorder::new();
    let client = client(&server, &recorder);

    let template = ExportTemplate::new("x");
    let _ = client.list_policies().await;
    let _ = client.get_policies(["P1"]).with_tax().await;
    let _ = client.create_policy("Ops").await;
    let _ = client
        .export_reports(&template, ReportsQuery::report_ids(["R1"]))
        .await;
    let _ = client.domain("acme.com").card_list().await;

    let jobs: Vec<String> = recorder
        .take()
        .iter()
        .map(|exchange| exchange.request().job_type().to_owned())
        .collect();
    assert_eq!(jobs, ["get", "get", "create", "file", "get"]);
}

/// An error response is the case you most want to see, so it is observed on
/// the same path as a success.
#[tokio::test]
async fn a_failed_call_is_observed_too() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 410,
        "responseMessage": "Required parameter 'policyName' is missing",
    })))
    .await;

    let recorder = Recorder::new();
    let err = client(&server, &recorder)
        .create_policy("Ops")
        .await
        .expect_err("410 in the body is a failure");
    assert!(matches!(err, Error::Api(_)), "{err:?}");

    let exchange = recorder.take().pop().expect("one exchange");
    // HTTP 200 carrying a body-level 410 — the reason raw bodies are worth
    // seeing at all.
    assert_eq!(exchange.status(), 200);
    assert!(exchange.body_text().contains("410"));
}

/// A download body is file content, not an envelope, and is observed byte for
/// byte.
#[tokio::test]
async fn a_download_body_is_observed_verbatim() {
    let server =
        server_with(ResponseTemplate::new(200).set_body_bytes(b"a,b,c\n1,2,3\n".to_vec())).await;

    let recorder = Recorder::new();
    let file = expensify::ExportedFile::from_parts(
        "export_1.csv",
        expensify::FileSystem::IntegrationServer,
    );
    let bytes = client(&server, &recorder)
        .download(&file)
        .await
        .expect("the mock answers file content");

    let exchange = recorder.take().pop().expect("one exchange");
    assert_eq!(exchange.body(), &bytes);
}

/// Off by default, and the observer is what turns it on — not a log level
/// somewhere else in the process.
#[tokio::test]
async fn nothing_is_recorded_without_an_observer() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200, "policyList": [] })),
    )
    .await;

    let unused = Recorder::new();
    let client = Client::builder(Credentials::new("partner-id", "partner-secret"))
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .build();

    client.list_policies().await.expect("the mock answers 200");
    assert!(unused.is_empty());
}

#[derive(Default)]
struct Trace(Arc<Mutex<Vec<String>>>);

impl Observer for Trace {
    fn on_request(&self, request: &ObservedRequest) {
        self.push(format!("request {}", request.job_type()));
    }

    fn on_exchange(&self, exchange: &Exchange) {
        self.push(format!("exchange {}", exchange.status().as_u16()));
    }
}

impl Trace {
    fn push(&self, line: String) {
        match self.0.lock() {
            Ok(mut lines) => lines.push(line),
            Err(poisoned) => poisoned.into_inner().push(line),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.0.lock().map(|lines| lines.clone()).unwrap_or_default()
    }
}

impl Clone for Trace {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

#[tokio::test]
async fn the_request_is_reported_before_the_response() {
    let server = server_with(
        ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200, "policyList": [] })),
    )
    .await;

    let trace = Trace::default();
    Client::builder(Credentials::new("partner-id", "partner-secret"))
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .observe(trace.clone())
        .build()
        .list_policies()
        .await
        .expect("the mock answers 200");

    assert_eq!(trace.lines(), ["request get", "exchange 200"]);
}

/// The reason [`Observer::on_request`] exists: a request that never comes back
/// is exactly when you want to know what went out.
#[tokio::test]
async fn a_request_that_never_answers_is_still_reported() {
    let trace = Trace::default();
    let err = Client::builder(Credentials::new("partner-id", "partner-secret"))
        // Port 1 is reserved and nothing listens on it.
        .base_url(Url::parse("http://127.0.0.1:1/").expect("a valid URL"))
        .no_rate_limiting()
        .observe(trace.clone())
        .build()
        .list_policies()
        .await
        .expect_err("nothing is listening");

    assert!(matches!(err, Error::Transport(_)), "{err:?}");
    assert_eq!(trace.lines(), ["request get"]);
}

/// The fixture story, end to end: record a live response once, replay it
/// forever. Nothing here asserts our own inference back at us — the second
/// half is driven by bytes the first half received.
#[tokio::test]
async fn a_recorded_body_replays_as_a_fixture() {
    let live = server_with(ResponseTemplate::new(200).set_body_json(json!({
        "responseCode": 200,
        "policyList": [
            { "id": "P1", "name": "Ops", "owner": "ops@acme.com", "role": "admin",
              "outputCurrency": "USD", "type": "corporate" },
        ],
    })))
    .await;

    let recorder = Recorder::new();
    let recorded = client(&live, &recorder)
        .list_policies()
        .await
        .expect("the live server answers");

    // What a fixture file would hold: the raw bytes, plus the job they answer.
    let exchange = recorder.take().pop().expect("one exchange");
    let fixture = exchange.body().clone();
    assert_eq!(exchange.request().job_type(), "get");

    let replay = server_with(
        ResponseTemplate::new(exchange.status().as_u16()).set_body_bytes(fixture.to_vec()),
    )
    .await;
    let replayed = Client::builder(Credentials::new("partner-id", "partner-secret"))
        .base_url(Url::parse(&replay.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .build()
        .list_policies()
        .await
        .expect("the fixture answers the same way");

    assert_eq!(replayed.len(), recorded.len());
    assert_eq!(replayed[0].name, recorded[0].name);
}
