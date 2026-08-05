//! End-to-end checks that need the real binary: help, usage errors, and the
//! exit codes scripts branch on. Nothing here reaches the network.

use std::process::{Command, Output};

/// Every level of the command tree. Also the list the PR body's help dump is
/// generated from.
const COMMAND_PATHS: &[&[&str]] = &[
    &[],
    &["auth"],
    &["auth", "login"],
    &["auth", "status"],
    &["auth", "logout"],
    &["get"],
    &["get", "policies"],
    &["get", "policy"],
    &["get", "cards"],
    &["export"],
    &["export", "reports"],
    &["export", "reconciliation"],
    &["download"],
    &["create"],
    &["create", "policy"],
    &["create", "expenses"],
    &["create", "report"],
    &["create", "expense-rule"],
    &["update"],
    &["update", "policy"],
    &["update", "tag-approvers"],
    &["update", "expense-rule"],
    &["update", "employees"],
    &["reimburse"],
    &["completion"],
    &["skill"],
    &["skill", "install"],
];

fn expensify(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_expensify"))
        // The credential resolver reads these; a developer's shell must not
        // decide what these tests exercise.
        .env_remove("EXPENSIFY_PARTNER_USER_ID")
        .env_remove("EXPENSIFY_PARTNER_USER_SECRET")
        .args(args)
        .output()
        .expect("running the built binary")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the process was not signalled")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn every_level_of_the_tree_has_help() {
    for path in COMMAND_PATHS {
        let mut args = path.to_vec();
        args.push("--help");
        let output = expensify(&args);
        assert_eq!(code(&output), 0, "`{}` has no help", args.join(" "));
        assert!(
            !output.stdout.is_empty(),
            "`{}` printed empty help",
            args.join(" ")
        );
    }
}

#[test]
fn the_root_help_documents_the_exit_codes() {
    let help = String::from_utf8_lossy(&expensify(&["--help"]).stdout).into_owned();
    assert!(help.contains("Exit codes:"), "{help}");
    assert!(help.contains("3   no usable credentials"), "{help}");
}

#[test]
fn an_unknown_subcommand_is_a_usage_error() {
    let output = expensify(&["frobnicate"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("unrecognized subcommand"),
        "{output:?}"
    );
}

/// Expensify rejects a policy read with no fields; so does the CLI, before
/// it goes near the network.
#[test]
fn reading_a_policy_needs_a_section() {
    let output = expensify(&["get", "policy", "P1"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("--with-categories"), "{output:?}");
}

/// The library's anchored query constructors, enforced one layer up.
#[test]
fn an_export_needs_a_selection_anchor() {
    let output = expensify(&["export", "reports", "--template", "t.ftl"]);
    assert_eq!(code(&output), 2);
    let stderr = stderr(&output);
    assert!(stderr.contains("--report-id"), "{stderr}");

    let output = expensify(&[
        "export",
        "reports",
        "--template",
        "t.ftl",
        "--since",
        "2026-07-01",
        "--report-id",
        "R1",
    ]);
    assert_eq!(code(&output), 2, "two anchors is a usage error");
}

#[test]
fn reimbursement_windows_need_a_start() {
    let output = expensify(&["reimburse", "--report-id", "R1", "--until", "2026-07-31"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("--since"), "{output:?}");
}

/// "At least one of these flags" is not an `ArgGroup` clap can enforce here,
/// but it is still a usage error, and it must exit like one.
#[test]
fn hand_checked_constraints_exit_as_usage_errors() {
    for args in [
        vec!["update", "policy", "P1"],
        vec!["update", "tag-approvers", "--policy-id", "P1"],
        vec!["create", "expenses"],
        vec![
            "create",
            "expense-rule",
            "--policy-id",
            "P1",
            "--employee-email",
            "a@acme.com",
        ],
    ] {
        let output = expensify(&args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
    }
}

/// Expensify answers 410 without `employeeEmail` and does not fall back to
/// the credential owner, so the flag is required rather than a nicety.
#[test]
fn creating_expenses_requires_an_employee() {
    let output = expensify(&[
        "create",
        "expenses",
        "--merchant",
        "Cloud Hosting Inc",
        "--date",
        "2026-07-31",
        "--amount-cents",
        "12900",
    ]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("--employee-email"), "{output:?}");
}

#[test]
fn a_bad_date_names_the_expected_format() {
    let output = expensify(&["reimburse", "--since", "07/01/2026"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("YYYY-MM-DD"), "{output:?}");
}

/// Half a credential pair is "no usable credentials", not a generic failure:
/// this is the code a CI job branches on.
#[test]
fn half_a_credential_pair_exits_three() {
    let output = expensify(&["--partner-user-id", "only-the-id", "get", "policies"]);
    assert_eq!(code(&output), 3);
    let stderr = stderr(&output);
    assert!(stderr.contains("--partner-user-secret"), "{stderr}");
    assert!(
        !stderr.contains("only-the-id"),
        "the id is echoed, fine, but check the pairing message: {stderr}"
    );
}

#[test]
fn completions_are_generated_for_each_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = expensify(&["completion", shell]);
        assert_eq!(code(&output), 0, "no completion for {shell}");
        let script = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(script.contains("expensify"), "{shell}: {script}");
    }
}

/// A withheld operation must not look available.
#[test]
fn pdf_export_is_not_offered() {
    let help =
        String::from_utf8_lossy(&expensify(&["export", "reports", "--help"]).stdout).into_owned();
    assert!(help.contains("csv"), "{help}");
    assert!(!help.contains("pdf"), "{help}");

    let output = expensify(&[
        "export",
        "reports",
        "--template",
        "t.ftl",
        "--since",
        "2026-07-01",
        "--format",
        "pdf",
    ]);
    assert_eq!(code(&output), 2);
}

/// The skill installer is the one command that writes to the filesystem, so
/// the whole path is exercised end to end: no credentials, no network, and no
/// silent overwrite.
#[test]
fn installing_the_skill_never_clobbers_silently() {
    let root = std::env::temp_dir().join(format!("expensify-cli-skill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let root = root.to_str().expect("a UTF-8 temp path");
    let path = std::path::Path::new(root)
        .join("expensify")
        .join("SKILL.md");

    let printed = expensify(&["skill", "install", "--print"]);
    assert_eq!(code(&printed), 0);
    assert!(!path.exists(), "--print must install nothing");

    let installed = expensify(&["skill", "install", "--skills-dir", root, "-o", "json"]);
    assert_eq!(code(&installed), 0, "{}", stderr(&installed));
    let written = std::fs::read_to_string(&path).expect("the skill was not written");
    assert_eq!(
        written.as_bytes(),
        printed.stdout,
        "--print differs from --install"
    );
    assert!(
        String::from_utf8_lossy(&installed.stdout).contains(&path.display().to_string()),
        "the installed path must be reported: {installed:?}"
    );

    std::fs::write(&path, "edited by hand").unwrap();
    let refused = expensify(&["skill", "install", "--skills-dir", root]);
    assert_eq!(code(&refused), 2, "a second install must be a usage error");
    assert!(stderr(&refused).contains("--force"), "{}", stderr(&refused));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited by hand");

    let forced = expensify(&["skill", "install", "--skills-dir", root, "--force"]);
    assert_eq!(code(&forced), 0, "{}", stderr(&forced));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tags_can_only_be_replaced_never_merged() {
    let help =
        String::from_utf8_lossy(&expensify(&["update", "policy", "--help"]).stdout).into_owned();
    assert!(help.contains("--tags"), "{help}");
    assert!(!help.contains("--merge-tags"), "{help}");
}

/// The verbosity ladder, exercised against a closed port: the request is
/// reported before anything can come back, which is the case `-v` exists for.
/// Nothing is listening on port 1, so this reaches the network stack and no
/// further.
#[test]
fn verbose_reports_the_request_before_the_failure() {
    let args = &[
        "--partner-user-id",
        "test-id",
        "--partner-user-secret",
        "test-secret-not-real",
        "--endpoint",
        "http://127.0.0.1:1/",
        "get",
        "policies",
    ];

    let quiet = expensify(args);
    assert_eq!(code(&quiet), 9, "{}", stderr(&quiet));
    assert!(
        !stderr(&quiet).contains("job=\"get\""),
        "silent by default: {}",
        stderr(&quiet)
    );

    let mut verbose = vec!["-v"];
    verbose.extend_from_slice(args);
    let verbose = expensify(&verbose);
    assert_eq!(code(&verbose), 9, "{}", stderr(&verbose));
    let output = stderr(&verbose);
    assert!(output.contains("request"), "{output}");
    assert!(output.contains("get"), "{output}");
    // No secret, at any level.
    assert!(!output.contains("test-secret-not-real"), "{output}");

    let mut louder = vec!["-vv"];
    louder.extend_from_slice(args);
    let louder = expensify(&louder);
    let output = stderr(&louder);
    // -vv is where the body of the request appears, and where the warning
    // about what a response body may contain has to appear with it.
    assert!(output.contains("requestJobDescription="), "{output}");
    assert!(output.contains("personal data"), "{output}");
    assert!(output.contains("<redacted>"), "{output}");
    assert!(!output.contains("test-secret-not-real"), "{output}");
}

/// The levels are a documented interface, including the warning about what
/// the deepest one prints.
#[test]
fn the_verbosity_levels_are_documented() {
    let help = String::from_utf8_lossy(&expensify(&["--help"]).stdout).into_owned();
    for expected in ["-vv ", "-vvv ", "personal data", "credentials redacted"] {
        assert!(help.contains(expected), "{help}");
    }
}

// ---- a server that answers wrongly on purpose -----------------------

/// One request, one canned response, then the socket closes.
///
/// The whole request is drained before answering: closing on a half-written
/// POST would surface as a transport failure, which is precisely the class of
/// error these tests need to *not* get.
fn answering(status: &str, content_type: &str, body: &'static str) -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("binding a port");
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );

    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(stream.try_clone().expect("cloning the socket"));
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {}
                Err(_) => return,
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            if line == "\r\n" {
                break;
            }
        }
        let _ = reader.take(length as u64).read_to_end(&mut Vec::new());

        let mut stream = stream;
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });

    endpoint
}

fn against(endpoint: &str, args: &[&str]) -> Output {
    let mut argv = vec![
        "--partner-user-id",
        "aa_test_account_example_com",
        "--partner-user-secret",
        "test-secret-not-real",
        "--endpoint",
        endpoint,
    ];
    argv.extend_from_slice(args);
    expensify(&argv)
}

fn line_starting(haystack: &str, prefix: &str) -> String {
    haystack
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{haystack}"))
        .to_owned()
}

/// The reason the fingerprint exists: an unreadable response is the defect
/// worth filing, and it has to arrive with a token to file it under.
#[test]
fn an_unreadable_response_is_fingerprinted() {
    let endpoint = answering("200 OK", "application/json", "this is not JSON");
    let output = against(&endpoint, &["get", "policies"]);
    let stderr = stderr(&output);

    assert_eq!(code(&output), 10, "{stderr}");
    assert_eq!(
        line_starting(&stderr, "defect fingerprint:"),
        "defect fingerprint: EXP-BAE7E423  [get.policies exit=10 decode.json]",
        "{stderr}"
    );
}

/// Stability is the property being bought, so it is checked across separate
/// processes rather than inside one.
#[test]
fn one_defect_fingerprints_the_same_way_every_run() {
    let first = answering("200 OK", "application/json", "this is not JSON");
    let second = answering(
        "200 OK",
        "text/plain",
        "also not JSON, and typed differently",
    );

    // Same defect, different message, different content-type, different run.
    assert_eq!(
        line_starting(&stderr(&against(&first, &["get", "policies"])), "defect"),
        line_starting(&stderr(&against(&second, &["get", "policies"])), "defect"),
    );
}

/// Two real defects must not land on one issue.
#[test]
fn different_defects_fingerprint_differently() {
    let unreadable = answering("200 OK", "application/json", "this is not JSON");
    let unplaceable = answering("502 Bad Gateway", "text/html", "<html>upstream</html>");
    let unrecognized = answering("200 OK", "application/json", r#"{"somethingElse":1}"#);

    let mut seen = std::collections::HashSet::new();
    for endpoint in [&unreadable, &unplaceable, &unrecognized] {
        let output = against(endpoint, &["get", "policies"]);
        assert_eq!(code(&output), 10, "{}", stderr(&output));
        assert!(
            seen.insert(line_starting(&stderr(&output), "defect fingerprint:")),
            "two failures share a fingerprint: {seen:?}"
        );
    }
}

/// A refusal, a usage error and a missing credential are not defects, and a
/// fingerprint on them would be an invitation to file one.
#[test]
fn explained_failures_carry_no_fingerprint() {
    let refused = answering(
        "200 OK",
        "application/json",
        r#"{"responseCode":403,"responseMessage":"no"}"#,
    );
    for output in [
        against(&refused, &["get", "policies"]),
        // No keychain read here on purpose: half a pair fails before the
        // resolver reaches one, so this stays the same speed everywhere.
        expensify(&["--partner-user-id", "only-the-id", "get", "policies"]),
        expensify(&["frobnicate"]),
    ] {
        assert!(
            !stderr(&output).contains("defect fingerprint"),
            "{}",
            stderr(&output)
        );
    }
}

/// The skill has to decide whether a `-vv` body is safe to paste into a public
/// issue, and the only thing that decides it is which account produced it.
/// A second command to find out is a second command that may not be safe to
/// run, so the failure itself has to say.
#[test]
fn a_failure_names_the_account_it_came_from() {
    let endpoint = answering("200 OK", "application/json", "this is not JSON");
    let stderr = stderr(&against(&endpoint, &["get", "policies"]));

    assert_eq!(
        line_starting(&stderr, "account:"),
        "account: aa_test_account_example_com (from command-line flags)",
        "{stderr}"
    );
    assert!(!stderr.contains("test-secret-not-real"), "{stderr}");
}

/// Including on a failure that is nobody's defect — the account is a fact
/// about the run, not about the diagnosis.
#[test]
fn the_account_is_named_even_where_no_fingerprint_is() {
    let output = against("http://127.0.0.1:1/", &["get", "policies"]);
    let stderr = stderr(&output);
    assert_eq!(code(&output), 9, "{stderr}");
    assert!(
        stderr.contains("account: aa_test_account_example_com"),
        "{stderr}"
    );
    assert!(!stderr.contains("defect fingerprint"), "{stderr}");
}

/// A credential failure has no account to name, and saying so would be a
/// guess about which half was wrong.
#[test]
fn a_credential_failure_names_no_account() {
    let output = expensify(&["--partner-user-id", "only-the-id", "get", "policies"]);
    assert_eq!(code(&output), 3);
    assert!(!stderr(&output).contains("account:"), "{}", stderr(&output));
}

/// "May contain personal data" is true of every account and therefore decides
/// nothing; which account is what decides whether to redact.
#[test]
fn the_verbose_warning_names_the_account_whose_data_it_prints() {
    let output = against("http://127.0.0.1:1/", &["-vv", "get", "policies"]);
    let stderr = stderr(&output);
    assert!(stderr.contains("personal data"), "{stderr}");
    assert!(stderr.contains("aa_test_account_example_com"), "{stderr}");
    assert!(!stderr.contains("test-secret-not-real"), "{stderr}");
}
