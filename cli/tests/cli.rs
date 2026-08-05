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
