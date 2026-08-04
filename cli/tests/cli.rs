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

#[test]
fn tags_can_only_be_replaced_never_merged() {
    let help =
        String::from_utf8_lossy(&expensify(&["update", "policy", "--help"]).stdout).into_owned();
    assert!(help.contains("--tags"), "{help}");
    assert!(!help.contains("--merge-tags"), "{help}");
}
