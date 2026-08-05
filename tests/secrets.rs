//! Secrets, from both directions.
//!
//! Redaction is for humans and serialization is for Expensify, and getting
//! those backwards fails in opposite ways: a leak that nobody notices, or
//! every call rejected. Both halves are asserted here over the public API, so
//! a new secret-bearing field is covered by construction rather than by
//! whoever adds it remembering to write a `Debug` impl.

use expensify::{
    Client, Credentials, EmployeeSource, Exchange, MaskedUrl, OnFinish, Recorder, ReportsQuery,
    Secret, SftpConnection, Url,
};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One string, so a leak anywhere is one `contains` away from being caught.
const SENTINEL: &str = "hunter2-super-secret-sentinel";

fn sftp() -> SftpConnection {
    SftpConnection {
        host: "sftp.acme.com".into(),
        login: "acme".into(),
        password: SENTINEL.into(),
        port: 22,
    }
}

fn feed() -> EmployeeSource {
    EmployeeSource::FetchUrl {
        url: format!("https://hr:{SENTINEL}@hr.acme.com/feed.json").into(),
        user: Some("hr".into()),
        password: Some(SENTINEL.into()),
    }
}

fn credentials() -> Credentials {
    Credentials::new("partner-id", SENTINEL)
}

fn client() -> Client {
    Client::builder(credentials())
        .base_url(
            format!("https://proxy:{SENTINEL}@gw.acme.com/expensify")
                .parse()
                .expect("a valid URL"),
        )
        .build()
}

/// Every public value that holds a secret, rendered every way the public API
/// renders values. Add a secret-bearing type and it belongs in this list —
/// but forgetting costs nothing, because the type itself already redacts.
#[test]
fn nothing_public_prints_a_secret() {
    let secret: Secret<String> = SENTINEL.into();
    let masked = MaskedUrl::from(format!("https://hr:{SENTINEL}@hr.acme.com/feed.json"));

    let rendered = [
        format!("{secret:?}"),
        format!("{secret}"),
        format!("{masked:?}"),
        format!("{masked}"),
        format!("{:?}", credentials()),
        format!("{:#?}", credentials()),
        format!("{:?}", client()),
        format!("{:?}", sftp()),
        format!("{:#?}", sftp()),
        format!("{:?}", OnFinish::sftp_upload(sftp())),
        format!("{:?}", feed()),
        format!("{:#?}", feed()),
        format!(
            "{:?}",
            EmployeeSource::Sftp {
                connection: sftp(),
                filename: "employees.json".into(),
            }
        ),
    ];

    for output in rendered {
        assert!(!output.contains(SENTINEL), "leaked: {output}");
    }
}

/// The other half of the contract. A redaction that also applied to the
/// request body would fail every call, and it would fail them quietly —
/// Expensify answers 410 with a message about credentials, not about this.
#[tokio::test]
async fn the_wire_carries_the_real_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200 })))
        .mount(&server)
        .await;

    let client = Client::builder(credentials())
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .build();

    client
        .update_employees(EmployeeSource::Sftp {
            connection: sftp(),
            filename: "employees.json".into(),
        })
        .await
        .expect("the mock answers 200");

    let requests = server.received_requests().await.expect("recording is on");
    let job = job_description(&requests[0].body);

    assert_eq!(job["credentials"]["partnerUserSecret"], SENTINEL);
    assert_eq!(job["credentials"]["sftp"]["password"], SENTINEL);
    assert_eq!(job["credentials"]["partnerUserID"], "partner-id");
}

#[tokio::test]
async fn a_feed_password_reaches_the_wire_and_not_the_observer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "responseCode": 200 })))
        .mount(&server)
        .await;

    let recorder = Recorder::new();
    let client = Client::builder(credentials())
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .observe(recorder.clone())
        .build();

    client
        .update_employees(feed())
        .await
        .expect("the mock answers 200");

    let requests = server.received_requests().await.expect("recording is on");
    let sent = job_description(&requests[0].body);
    assert_eq!(sent["credentials"]["feedPassword"], SENTINEL);
    // The feed URL is a credential carrier, and the *server* needs it whole.
    assert_eq!(
        sent["credentials"]["feedUrl"],
        format!("https://hr:{SENTINEL}@hr.acme.com/feed.json")
    );

    let exchanges = recorder.take();
    let observed = exchanges[0].request().job_description();
    assert!(!observed.contains(SENTINEL), "{observed}");
    // The URL is masked rather than dropped: which host the feed came from is
    // half of any diagnosis, and it is not the secret part.
    assert!(observed.contains("hr.acme.com"), "{observed}");
    assert!(observed.contains("<redacted>"), "{observed}");
}

/// The export path carries a third secret (the `sftpUpload` password) through
/// a different part of the job description.
#[tokio::test]
async fn an_observed_export_shows_the_shape_and_none_of_the_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "responseCode": 200, "filename": "export_1.csv" })),
        )
        .mount(&server)
        .await;

    let recorder = Recorder::new();
    let client = Client::builder(credentials())
        .base_url(Url::parse(&server.uri()).expect("wiremock hands back a valid URL"))
        .no_rate_limiting()
        .observe(recorder.clone())
        .build();

    let template = expensify::ExportTemplate::new("<#list reports as r></#list>");
    client
        .export_reports(&template, ReportsQuery::report_ids(["R1"]))
        .on_finish(OnFinish::sftp_upload(sftp()))
        .await
        .expect("the mock answers a filename");

    let exchanges = recorder.take();
    let request = exchanges[0].request();

    for (name, value) in request.fields() {
        assert!(!value.contains(SENTINEL), "leaked in `{name}`: {value}");
    }
    // Redacting must not blank the diagnostic content around it.
    assert!(request.job_description().contains("sftp.acme.com"));
    assert!(request.job_description().contains("R1"));
    assert_eq!(request.job_type(), "file");
    assert!(
        request
            .field("template")
            .is_some_and(|t| t.contains("reports")),
        "the template field rides along"
    );

    // Nor may the rendered forms of the exchange itself carry one.
    let exchange: &Exchange = &exchanges[0];
    for output in [format!("{exchange}"), format!("{exchange:?}")] {
        assert!(!output.contains(SENTINEL), "leaked: {output}");
    }
}

/// A proxy `base_url` is caller-supplied and can carry userinfo; `Url`'s own
/// `Debug` prints it verbatim, so nothing may print a bare `Url`.
#[test]
fn a_proxy_password_in_the_endpoint_is_masked() {
    let rendered = format!("{:?}", client());
    assert!(!rendered.contains(SENTINEL), "{rendered}");
    assert!(rendered.contains("<redacted>@gw.acme.com"), "{rendered}");
    assert!(rendered.contains("/expensify"), "{rendered}");
}

fn job_description(body: &[u8]) -> Value {
    let form: Vec<(String, String)> =
        serde_urlencoded::from_bytes(body).expect("body is form-urlencoded");
    let raw = form
        .into_iter()
        .find(|(key, _)| key == "requestJobDescription")
        .map(|(_, value)| value)
        .expect("every job carries a description");
    serde_json::from_str(&raw).expect("the description is JSON")
}
