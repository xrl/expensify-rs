//! The whole command tree. Kept in one module so the surface can be read
//! top to bottom.

use clap::{Args, Parser, Subcommand, ValueEnum};
use expensify::Secret;
use time::Date;

use crate::output::OutputFormat;
use crate::spec::parse_date;

/// So a secret typed on the command line is a `Secret` from the moment clap
/// parses it, rather than a `String` that `Debug` would print.
fn secret(raw: &str) -> Result<Secret<String>, std::convert::Infallible> {
    Ok(Secret::new(raw.to_owned()))
}

/// Exit codes, documented in `--help` because scripts branch on them.
const EXIT_CODES: &str = "\
Exit codes:
  0   success
  1   unexpected failure
  2   usage error
  3   no usable credentials
  4   permission denied by Expensify
  5   not found
  6   request rejected as invalid
  7   rate limited
  8   partial success (some items skipped or failed)
  9   network failure
  10  unreadable response from Expensify";

#[derive(Debug, Parser)]
#[command(
    name = "expensify",
    version,
    about = "Command-line client for the Expensify Integration Server API",
    long_about = "Command-line client for the Expensify Integration Server API.\n\n\
                  Commands are verb-noun and share their flags: `-o/--output` picks the \
                  format, credentials resolve the same way everywhere, and every read \
                  command can emit JSON for scripting.",
    after_long_help = EXIT_CODES,
    max_term_width = 100
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Flags every subcommand accepts. Their own heading, so they do not
/// interleave with a command's own options.
#[derive(Debug, Args)]
#[command(next_help_heading = "Global options")]
pub struct GlobalArgs {
    /// Output format
    #[arg(
        short = 'o',
        long,
        global = true,
        value_name = "FORMAT",
        default_value = "table",
        value_enum
    )]
    pub output: OutputFormat,

    /// Partner user ID, overriding the environment and the keychain [env:
    /// EXPENSIFY_PARTNER_USER_ID]
    #[arg(long, global = true, value_name = "ID")]
    pub partner_user_id: Option<String>,

    /// Partner user secret, overriding the environment and the keychain [env:
    /// EXPENSIFY_PARTNER_USER_SECRET]
    #[arg(long, global = true, value_name = "SECRET", value_parser = secret)]
    pub partner_user_secret: Option<Secret<String>>,

    /// Post to a different Integration Server (testing, proxies)
    #[arg(long, global = true, value_name = "URL")]
    pub endpoint: Option<String>,

    /// Disable the built-in 5-per-10s / 20-per-60s rate limiter
    #[arg(long, global = true)]
    pub no_rate_limit: bool,

    /// Log CLI activity to stderr; repeat for more detail
    ///
    /// -v    one line per API call: job type, endpoint, status, size, timing.
    /// -vv   the whole exchange — request body as sent (credentials redacted)
    ///       and the response body verbatim, with its content-type. Response
    ///       bodies routinely contain personal data (employee names, email
    ///       addresses, card numbers); treat the output accordingly.
    /// -vvv  adds HTTP transport tracing from the underlying libraries, for
    ///       when the suspect is the connection rather than the payload.
    #[arg(short, long, global = true, action = clap::ArgAction::Count, verbatim_doc_comment)]
    pub verbose: u8,

    /// Suppress progress notes on stderr
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage the stored partner credentials
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },

    /// Read policies, cards and other Expensify state
    Get {
        #[command(subcommand)]
        command: GetCommand,
    },

    /// Start an export job; prints the file handle to download
    Export {
        #[command(subcommand)]
        command: ExportCommand,
    },

    /// Download a file produced by an export job
    Download(DownloadArgs),

    /// Create policies, expenses, reports and expense rules
    Create {
        #[command(subcommand)]
        command: CreateCommand,
    },

    /// Update policies, tag approvers, expense rules and employees
    Update {
        #[command(subcommand)]
        command: UpdateCommand,
    },

    /// Mark approved reports as reimbursed
    Reimburse(ReimburseArgs),

    /// Print a shell completion script
    Completion(CompletionArgs),

    /// Install the Claude Code agent skill bundled with this binary
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

// ---- auth -----------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a partner credential pair in the OS keychain
    #[command(long_about = "Store a partner credential pair in the OS keychain.\n\n\
                      Generate the pair at https://www.expensify.com/tools/integrations/ \
                      — Expensify shows the secret exactly once. The secret is read \
                      without echo and is never written to a file.")]
    Login(LoginArgs),

    /// Show which credentials would be used, and where they come from
    Status,

    /// Remove the credentials from the OS keychain
    Logout,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Partner user ID; prompted for when absent
    #[arg(long, value_name = "ID")]
    pub partner_user_id: Option<String>,
}

// ---- get ------------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum GetCommand {
    /// List every policy the credentials can see
    Policies(GetPoliciesArgs),

    /// Read selected sections of one or more policies
    #[command(long_about = "Read selected sections of one or more policies.\n\n\
                      At least one --with-* flag is required: Expensify rejects a \
                      request that names no fields, and each flag costs a section of \
                      response. --with-employees and --with-tax need policy-admin \
                      credentials.")]
    Policy(GetPolicyArgs),

    /// List the company cards on a domain
    Cards(GetCardsArgs),
}

#[derive(Debug, Args)]
pub struct GetPoliciesArgs {
    /// Only policies where the credential owner is an admin
    #[arg(long)]
    pub admin_only: bool,

    /// Act on behalf of another user; needs a third-party access grant
    #[arg(long, value_name = "EMAIL")]
    pub on_behalf_of: Option<String>,
}

#[derive(Debug, Args)]
pub struct GetPolicyArgs {
    /// Policy IDs to read
    #[arg(value_name = "POLICY_ID", required = true)]
    pub policy_ids: Vec<String>,

    #[command(flatten)]
    pub sections: PolicySections,

    /// Act on behalf of another user; needs a third-party access grant
    #[arg(long, value_name = "EMAIL")]
    pub on_behalf_of: Option<String>,
}

/// Expensify rejects a policy read that names no fields, so at least one of
/// these is required — and each one it gets flips a compile-time flag in the
/// library's response type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Args)]
#[group(id = "policy_sections", required = true, multiple = true)]
pub struct PolicySections {
    /// Include expense categories
    #[arg(long)]
    pub with_categories: bool,

    /// Include report fields
    #[arg(long)]
    pub with_report_fields: bool,

    /// Include tags
    #[arg(long)]
    pub with_tags: bool,

    /// Include the tax configuration
    #[arg(long)]
    pub with_tax: bool,

    /// Include policy members
    #[arg(long)]
    pub with_employees: bool,
}

#[derive(Debug, Args)]
pub struct GetCardsArgs {
    /// Domain to list cards for; needs domain-admin credentials
    #[arg(long, value_name = "DOMAIN")]
    pub domain: String,
}

// ---- export ---------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum ExportCommand {
    /// Export reports through a FreeMarker template
    #[command(long_about = "Export reports through a FreeMarker template.\n\n\
                      Rendering continues server-side after this returns, so the printed \
                      handle may not be downloadable immediately. Expensify publishes no \
                      ready signal; retry `expensify download`.")]
    Reports(ExportReportsArgs),

    /// Reconcile a domain's card transactions through a FreeMarker template
    #[command(
        long_about = "Reconcile a domain's card transactions through a FreeMarker \
                      template.\n\nThis job runs synchronously, so the printed handle is \
                      immediately downloadable. Needs domain-admin credentials."
    )]
    Reconciliation(ReconcileArgs),
}

#[derive(Debug, Args)]
pub struct ExportReportsArgs {
    /// FreeMarker template file, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub template: String,

    #[command(flatten)]
    pub anchor: ReportAnchor,

    /// End of the window; only with --since or --approved-after
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub until: Option<Date>,

    /// Restrict to these policies
    #[arg(long = "policy-id", value_name = "ID")]
    pub policy_ids: Vec<String>,

    /// Skip reports already marked exported under this label
    #[arg(long, value_name = "LABEL")]
    pub not_exported_as: Option<String>,

    /// Restrict to these report states; repeatable
    #[arg(long = "state", value_name = "STATE", value_enum)]
    pub states: Vec<ReportStateArg>,

    /// Cap the number of exported reports
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Export one employee's reports; needs domain access to that employee
    #[arg(long, value_name = "EMAIL")]
    pub employee_email: Option<String>,

    /// Output file format
    #[arg(long, value_name = "FORMAT", value_enum, default_value = "csv")]
    pub format: ExportFormatArg,

    /// Filename stem; Expensify appends a random suffix regardless
    #[arg(long, value_name = "NAME")]
    pub basename: Option<String>,

    /// On success, mark the exported reports with this label
    #[arg(long, value_name = "LABEL")]
    pub mark_as_exported: Option<String>,

    /// On success, email these comma-separated recipients
    #[arg(long, value_name = "RECIPIENTS")]
    pub email: Option<String>,

    /// Body of the notification email
    #[arg(long, value_name = "TEXT", requires = "email")]
    pub email_message: Option<String>,

    /// Run the export without firing any on-finish action
    #[arg(long)]
    pub test_run: bool,
}

/// Expensify requires exactly one selection anchor, mirroring the library's
/// anchored `ReportsQuery` constructors.
#[derive(Debug, Args)]
#[group(id = "report_anchor", required = true, multiple = false)]
pub struct ReportAnchor {
    /// Export these reports
    #[arg(long = "report-id", value_name = "ID")]
    pub report_ids: Vec<String>,

    /// Export reports created on or after this date
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub since: Option<Date>,

    /// Export reports approved after this date
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub approved_after: Option<Date>,
}

#[derive(Debug, Args)]
pub struct ReconcileArgs {
    /// Domain to reconcile; needs domain-admin credentials
    #[arg(long, value_name = "DOMAIN")]
    pub domain: String,

    /// FreeMarker template file, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub template: String,

    /// Start of the window
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub start: Date,

    /// End of the window
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub end: Date,

    /// Which transactions to include
    #[arg(long, value_name = "SCOPE", value_enum, default_value = "unreported")]
    pub scope: ReconciliationScopeArg,

    /// Restrict to one card feed; default is every feed
    #[arg(long, value_name = "FEED")]
    pub feed: Option<String>,

    /// Output file format
    #[arg(long, value_name = "FORMAT", value_enum, default_value = "csv")]
    pub format: ReconciliationFormatArg,

    /// On success, email these comma-separated recipients
    #[arg(long, value_name = "RECIPIENTS")]
    pub email_on_finish: Option<String>,
}

// ---- download -------------------------------------------------------

#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// Filename reported by `expensify export`
    #[arg(value_name = "FILENAME")]
    pub filename: String,

    /// Which store produced the file
    #[arg(
        long,
        value_name = "SYSTEM",
        value_enum,
        default_value = "integration-server"
    )]
    pub file_system: FileSystemArg,

    /// Write to this path instead of stdout
    #[arg(short = 'O', long, value_name = "PATH")]
    pub out: Option<String>,
}

// ---- create ---------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum CreateCommand {
    /// Create a policy
    Policy(CreatePolicyArgs),

    /// Create expenses, one from flags or many from a JSON file
    Expenses(CreateExpensesArgs),

    /// Create a report from expense lines
    #[command(
        long_about = "Create a report from expense lines.\n\nExpensify documents this \
                      as needing an unlock from support plus domain-admin rights, but \
                      it worked on a policy-admin account with neither. Treat the \
                      requirement as unconfirmed, not absent."
    )]
    Report(CreateReportArgs),

    /// Create an expense rule for one employee on one policy
    ExpenseRule(CreateExpenseRuleArgs),
}

#[derive(Debug, Args)]
pub struct CreatePolicyArgs {
    /// Name of the new policy
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Subscription plan
    #[arg(long, value_name = "PLAN", value_enum)]
    pub plan: Option<PolicyPlanArg>,
}

#[derive(Debug, Args)]
pub struct CreateExpensesArgs {
    /// JSON array of expenses, or `-` for stdin; excludes the inline flags
    #[arg(long, value_name = "FILE", conflicts_with_all = ["merchant", "date", "amount_cents"])]
    pub file: Option<String>,

    /// Merchant name
    #[arg(long, value_name = "NAME", requires_all = ["date", "amount_cents"])]
    pub merchant: Option<String>,

    /// Expense date
    #[arg(long, value_name = "DATE", value_parser = parse_date, requires_all = ["merchant", "amount_cents"])]
    pub date: Option<Date>,

    /// Amount in minor units: 12900 is $129.00
    #[arg(long, value_name = "CENTS", requires_all = ["merchant", "date"])]
    pub amount_cents: Option<i64>,

    /// Currency of --amount-cents
    #[arg(long, value_name = "CODE", default_value = "USD")]
    pub currency: String,

    /// Policy category name
    #[arg(long, value_name = "NAME")]
    pub category: Option<String>,

    /// Policy tag name
    #[arg(long, value_name = "NAME")]
    pub tag: Option<String>,

    /// Free-text comment
    #[arg(long, value_name = "TEXT")]
    pub comment: Option<String>,

    /// Caller-chosen unique ID, echoed back on export
    #[arg(long, value_name = "ID")]
    pub external_id: Option<String>,

    /// Attach to an existing report
    #[arg(long, value_name = "ID")]
    pub report_id: Option<String>,

    /// Policy the tax rate belongs to
    #[arg(long, value_name = "ID")]
    pub policy_id: Option<String>,

    /// Tax rate ID from `get policy --with-tax`
    #[arg(long, value_name = "ID")]
    pub tax_rate_id: Option<String>,

    /// Explicit tax amount for a partially taxed expense
    #[arg(long, value_name = "CENTS", requires = "tax_rate_id")]
    pub tax_amount_cents: Option<i64>,

    /// Mark billable to a client
    #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub billable: Option<bool>,

    /// Mark reimbursable to the employee
    #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub reimbursable: Option<bool>,

    /// Employee the expenses belong to; Expensify requires it
    #[arg(long, value_name = "EMAIL")]
    pub employee_email: String,
}

#[derive(Debug, Args)]
pub struct CreateReportArgs {
    /// Policy the report belongs to
    #[arg(long, value_name = "ID")]
    pub policy_id: String,

    /// Employee the report is created for
    #[arg(long, value_name = "EMAIL")]
    pub employee_email: String,

    /// Report title
    #[arg(long, value_name = "TITLE")]
    pub title: String,

    /// JSON array of expense lines, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub expenses: String,

    /// Report field as NAME=VALUE; repeatable
    #[arg(long = "field", value_name = "NAME=VALUE")]
    pub fields: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CreateExpenseRuleArgs {
    /// Policy the rule applies to
    #[arg(long, value_name = "ID")]
    pub policy_id: String,

    /// Employee the rule applies to
    #[arg(long, value_name = "EMAIL")]
    pub employee_email: String,

    /// Auto-apply this tag
    #[arg(long, value_name = "NAME")]
    pub tag: Option<String>,

    /// Auto-set the billable flag
    #[arg(long, value_name = "BOOL")]
    pub default_billable: Option<bool>,
}

// ---- update ---------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    /// Update a policy's categories, report fields or tags
    #[command(
        long_about = "Update a policy's categories, report fields or tags.\n\n\
                      Tags are replace-only: a tags update sent with action=merge was \
                      observed deleting every unlisted tag and reporting success, so \
                      this CLI offers no tag merge. Categories and report fields \
                      default to merging."
    )]
    Policy(UpdatePolicyArgs),

    /// Assign or clear the approver for policy tags
    TagApprovers(TagApproversArgs),

    /// Update an existing expense rule
    ExpenseRule(UpdateExpenseRuleArgs),

    /// Feed employee records into policies; needs domain-admin credentials
    Employees(UpdateEmployeesArgs),
}

#[derive(Debug, Args)]
pub struct UpdatePolicyArgs {
    /// Policies to update
    #[arg(value_name = "POLICY_ID", required = true)]
    pub policy_ids: Vec<String>,

    /// JSON array of categories, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub categories: Option<String>,

    /// Whether --categories adds to or replaces the policy's categories
    #[arg(
        long,
        value_name = "MODE",
        value_enum,
        default_value = "merge",
        requires = "categories"
    )]
    pub categories_mode: UpdateMode,

    /// JSON array of report field definitions, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub report_fields: Option<String>,

    /// Whether --report-fields adds to or replaces the policy's report fields
    #[arg(
        long,
        value_name = "MODE",
        value_enum,
        default_value = "merge",
        requires = "report_fields"
    )]
    pub report_fields_mode: UpdateMode,

    /// JSON array of tag levels replacing every tag on the policy
    #[arg(long, value_name = "FILE", conflicts_with = "tags_csv")]
    pub tags: Option<String>,

    /// CSV/TSV file of tags replacing every tag on the policy
    #[arg(long, value_name = "FILE")]
    pub tags_csv: Option<String>,

    /// Treat the CSV's tag levels as dependent
    #[arg(long, requires = "tags_csv")]
    pub tags_csv_dependent: bool,

    /// Whether tags are required: one value if dependent, one per level if not
    #[arg(long, value_name = "BOOL", requires = "tags_csv")]
    pub tags_csv_required: Vec<bool>,

    /// The CSV's last column holds GL codes
    #[arg(long, requires = "tags_csv")]
    pub tags_csv_gl_codes: bool,

    /// The CSV's first row is a header
    #[arg(long, requires = "tags_csv")]
    pub tags_csv_header_row: bool,

    /// The file is tab-separated
    #[arg(long, requires = "tags_csv")]
    pub tags_csv_tsv: bool,
}

#[derive(Debug, Args)]
pub struct TagApproversArgs {
    /// Policy whose tags are being routed
    #[arg(long, value_name = "ID")]
    pub policy_id: String,

    /// Route a tag to an approver, as TAG=EMAIL; repeatable
    #[arg(long = "assign", value_name = "TAG=EMAIL")]
    pub assign: Vec<String>,

    /// Remove a tag's approver; repeatable
    #[arg(long = "clear", value_name = "TAG")]
    pub clear: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateExpenseRuleArgs {
    /// Policy the rule applies to
    #[arg(long, value_name = "ID")]
    pub policy_id: String,

    /// Employee the rule applies to
    #[arg(long, value_name = "EMAIL")]
    pub employee_email: String,

    /// Rule to modify
    #[arg(long, value_name = "N")]
    pub rule_id: i64,

    /// Auto-apply this tag
    #[arg(long, value_name = "NAME")]
    pub tag: Option<String>,

    /// Auto-set the billable flag
    #[arg(long, value_name = "BOOL")]
    pub default_billable: Option<bool>,
}

#[derive(Debug, Args)]
pub struct UpdateEmployeesArgs {
    /// JSON array of employee records, or `-` for stdin
    #[arg(long, value_name = "FILE")]
    pub file: String,

    /// Report what would change without changing it
    #[arg(long)]
    pub dry_run: bool,

    /// Which employees get their primary policy set
    #[arg(long, value_name = "MODE", value_enum)]
    pub primary_policy: Option<PrimaryPolicyArg>,

    /// Leave broken approval chains alone; Expensify fixes them by default
    #[arg(long)]
    pub no_approval_chain_fixes: bool,

    /// Only import the first level of managers
    #[arg(long)]
    pub first_level_managers_only: bool,

    /// Do not email employees about the change
    #[arg(long)]
    pub skip_notification_emails: bool,

    /// On completion, email these comma-separated recipients
    #[arg(long, value_name = "RECIPIENTS")]
    pub email_on_finish: Option<String>,
}

// ---- reimburse ------------------------------------------------------

#[derive(Debug, Args)]
#[command(
    long_about = "Mark approved reports as reimbursed.\n\nApproved to Reimbursed is the \
                  only transition Expensify supports, so there is no status flag. By \
                  default a partially applied run is an error (exit 8); \
                  --tolerate-partial reports it as data instead."
)]
pub struct ReimburseArgs {
    #[command(flatten)]
    pub anchor: ReimburseAnchor,

    /// End of the window; only with --since
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub until: Option<Date>,

    /// Free-text payment label recorded on the reports
    #[arg(long, value_name = "SOURCE")]
    pub payment_source: Option<String>,

    /// Treat skipped and failed reports as data, not as an error
    #[arg(long)]
    pub tolerate_partial: bool,
}

/// Same anchoring rule as the exporter, and the same reason.
#[derive(Debug, Args)]
#[group(id = "reimburse_anchor", required = true, multiple = false)]
pub struct ReimburseAnchor {
    /// Reimburse these reports
    #[arg(long = "report-id", value_name = "ID")]
    pub report_ids: Vec<String>,

    /// Reimburse reports submitted or created on or after this date
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    pub since: Option<Date>,
}

// ---- completion -----------------------------------------------------

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate a completion script for
    #[arg(value_name = "SHELL", value_enum)]
    pub shell: clap_complete::Shell,
}

// ---- skill ----------------------------------------------------------

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Write the bundled skill into a Claude Code skills directory
    #[command(
        long_about = "Write the bundled skill into a Claude Code skills directory.\n\n\
                      The skill is compiled into this binary, so installing needs no \
                      checkout and no network. Nothing is overwritten without --force."
    )]
    Install(SkillInstallArgs),
}

#[derive(Debug, Args)]
pub struct SkillInstallArgs {
    /// Install into ./.claude/skills instead of the personal directory
    #[arg(long, conflicts_with = "skills_dir")]
    pub project: bool,

    /// Install into this skills directory, rather than a discovered one
    #[arg(long, value_name = "DIR")]
    pub skills_dir: Option<std::path::PathBuf>,

    /// Replace an already-installed SKILL.md
    #[arg(long)]
    pub force: bool,

    /// Write the skill to stdout and install nothing
    #[arg(long, conflicts_with_all = ["project", "skills_dir", "force"])]
    pub print: bool,
}

// ---- value enums ----------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ReportStateArg {
    Open,
    Submitted,
    Approved,
    Reimbursed,
    Archived,
}

/// No `pdf`: the library withholds it because Expensify emits one PDF per
/// report and a single file handle cannot name several files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExportFormatArg {
    Csv,
    Xls,
    Xlsx,
    Txt,
    Json,
    Xml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ReconciliationFormatArg {
    Csv,
    Txt,
    Json,
    Xml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ReconciliationScopeArg {
    /// Card transactions not yet on any report
    Unreported,
    /// Every card transaction in the window
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum FileSystemArg {
    IntegrationServer,
    Reconciliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PolicyPlanArg {
    Team,
    Corporate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum UpdateMode {
    /// Add to what the policy already has
    Merge,
    /// Delete everything not listed
    ReplaceAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PrimaryPolicyArg {
    None,
    NewEmployees,
    AllEmployees,
}

/// A stable name for the command that ran, for the defect fingerprint.
///
/// Deliberately not derived from `argv`: flags, IDs and dates would all end up
/// in it, and two runs of one command would fingerprint differently. These
/// strings are a wire format between releases — rename a command and the old
/// name stays here.
pub fn path(command: &Command) -> &'static str {
    match command {
        Command::Auth { command } => match command {
            AuthCommand::Login(_) => "auth.login",
            AuthCommand::Status => "auth.status",
            AuthCommand::Logout => "auth.logout",
        },
        Command::Get { command } => match command {
            GetCommand::Policies(_) => "get.policies",
            GetCommand::Policy(_) => "get.policy",
            GetCommand::Cards(_) => "get.cards",
        },
        Command::Export { command } => match command {
            ExportCommand::Reports(_) => "export.reports",
            ExportCommand::Reconciliation(_) => "export.reconciliation",
        },
        Command::Download(_) => "download",
        Command::Create { command } => match command {
            CreateCommand::Policy(_) => "create.policy",
            CreateCommand::Expenses(_) => "create.expenses",
            CreateCommand::Report(_) => "create.report",
            CreateCommand::ExpenseRule(_) => "create.expense-rule",
        },
        Command::Update { command } => match command {
            UpdateCommand::Policy(_) => "update.policy",
            UpdateCommand::TagApprovers(_) => "update.tag-approvers",
            UpdateCommand::ExpenseRule(_) => "update.expense-rule",
            UpdateCommand::Employees(_) => "update.employees",
        },
        Command::Reimburse(_) => "reimburse",
        Command::Completion(_) => "completion",
        Command::Skill { command } => match command {
            SkillCommand::Install(_) => "skill.install",
        },
    }
}

/// Report a constraint clap cannot express the way clap reports its own:
/// same styling, same usage line, same exit code.
///
/// `requires` cannot express these — an argument that belongs to a required
/// `ArgGroup` counts as satisfied by any other member of that group.
pub fn usage_error(message: impl std::fmt::Display) -> ! {
    use clap::CommandFactory as _;
    Cli::command()
        .error(clap::error::ErrorKind::ArgumentConflict, message)
        .exit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// Every leaf, because a leaf missing from `path` would fingerprint under
    /// its neighbour's name — and because parsing these proves the argvs the
    /// fingerprint scheme is described in terms of are real.
    #[test]
    fn every_leaf_has_its_own_stable_path() {
        let leaves: &[&[&str]] = &[
            &["auth", "login"],
            &["auth", "status"],
            &["auth", "logout"],
            &["get", "policies"],
            &["get", "policy", "P1", "--with-tags"],
            &["get", "cards", "--domain", "acme.com"],
            &[
                "export",
                "reports",
                "--template",
                "t.ftl",
                "--since",
                "2026-07-01",
            ],
            &[
                "export",
                "reconciliation",
                "--domain",
                "acme.com",
                "--template",
                "t.ftl",
                "--start",
                "2026-07-01",
                "--end",
                "2026-08-01",
            ],
            &["download", "export_8f3d.csv"],
            &["create", "policy", "Engineering"],
            &[
                "create",
                "expenses",
                "--employee-email",
                "a@acme.com",
                "--merchant",
                "M",
                "--date",
                "2026-07-01",
                "--amount-cents",
                "100",
            ],
            &[
                "create",
                "report",
                "--policy-id",
                "P1",
                "--employee-email",
                "a@acme.com",
                "--title",
                "T",
                "--expenses",
                "-",
            ],
            &[
                "create",
                "expense-rule",
                "--policy-id",
                "P1",
                "--employee-email",
                "a@acme.com",
                "--tag",
                "Core",
            ],
            &["update", "policy", "P1", "--categories", "-"],
            &[
                "update",
                "tag-approvers",
                "--policy-id",
                "P1",
                "--assign",
                "Core=a@acme.com",
            ],
            &[
                "update",
                "expense-rule",
                "--policy-id",
                "P1",
                "--employee-email",
                "a@acme.com",
                "--rule-id",
                "1",
                "--tag",
                "Core",
            ],
            &["update", "employees", "--file", "-"],
            &["reimburse", "--report-id", "R1"],
            &["completion", "bash"],
            &["skill", "install"],
        ];

        let mut seen = std::collections::HashSet::new();
        for leaf in leaves {
            let argv: Vec<_> = std::iter::once("expensify")
                .chain(leaf.iter().copied())
                .collect();
            let parsed = Cli::try_parse_from(&argv).unwrap_or_else(|err| panic!("{leaf:?}: {err}"));
            let named = path(&parsed.command);
            assert!(seen.insert(named), "{named} names more than one command");
        }
        assert_eq!(seen.len(), leaves.len());
    }

    #[test]
    fn dates_parse_as_iso() {
        assert!(parse_date("2026-07-01").is_ok());
        assert!(parse_date("07/01/2026").is_err());
        assert!(parse_date("2026-13-01").is_err());
    }
}
