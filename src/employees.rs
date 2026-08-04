//! Advanced Employee Updater (documented at `doc/employeeUpdater/`), plus
//! the deprecated CSV employee updater behind the
//! `employee-updater-deprecated` feature.

use std::collections::HashMap;

use crate::client::Client;
use crate::error::Error;
use crate::export::SftpConnection;
use crate::policy::PolicyRole;
use crate::types::PolicyId;
use crate::BoxFuture;

/// One employee record in the feed. Required fields in the constructor.
#[derive(Clone, Debug)]
pub struct Employee {
    employee_email: String,
    manager_email: String,
    employee_id: String,
    policy_id: PolicyId,
    first_name: Option<String>,
    last_name: Option<String>,
    custom_field_1: Option<String>,
    custom_field_2: Option<String>,
    approval_limit: Option<i64>,
    over_limit_approver: Option<String>,
    worker_status: Option<String>,
    is_terminated: Option<bool>,
    domain_group_id: Option<String>,
    approves_to: Option<String>,
    role: Option<PolicyRole>,
    additional_policy_ids: Vec<PolicyId>,
    remove_from_unassigned_policies: bool,
    default_tags: Vec<String>,
}

impl Employee {
    /// `employee_id` drives email-change detection: Expensify matches on
    /// it and merges accounts when the email differs. It also auto-fills
    /// Custom Field 1 unless overridden.
    pub fn new(
        employee_email: impl Into<String>,
        manager_email: impl Into<String>,
        employee_id: impl Into<String>,
        policy_id: impl Into<PolicyId>,
    ) -> Self {
        Self {
            employee_email: employee_email.into(),
            manager_email: manager_email.into(),
            employee_id: employee_id.into(),
            policy_id: policy_id.into(),
            first_name: None,
            last_name: None,
            custom_field_1: None,
            custom_field_2: None,
            approval_limit: None,
            over_limit_approver: None,
            worker_status: None,
            is_terminated: None,
            domain_group_id: None,
            approves_to: None,
            role: None,
            additional_policy_ids: Vec::new(),
            remove_from_unassigned_policies: false,
            default_tags: Vec::new(),
        }
    }

    pub fn first_name(mut self, name: impl Into<String>) -> Self {
        self.first_name = Some(name.into());
        self
    }

    pub fn last_name(mut self, name: impl Into<String>) -> Self {
        self.last_name = Some(name.into());
        self
    }

    pub fn custom_field_1(mut self, value: impl Into<String>) -> Self {
        self.custom_field_1 = Some(value.into());
        self
    }

    pub fn custom_field_2(mut self, value: impl Into<String>) -> Self {
        self.custom_field_2 = Some(value.into());
        self
    }

    pub fn approval_limit(mut self, limit: i64) -> Self {
        self.approval_limit = Some(limit);
        self
    }

    pub fn over_limit_approver(mut self, email: impl Into<String>) -> Self {
        self.over_limit_approver = Some(email.into());
        self
    }

    /// Free-text; "On Leave" removes the person from manager duty.
    pub fn worker_status(mut self, status: impl Into<String>) -> Self {
        self.worker_status = Some(status.into());
        self
    }

    pub fn terminated(mut self) -> Self {
        self.is_terminated = Some(true);
        self
    }

    /// Only applied when every employee in the feed carries one.
    pub fn domain_group_id(mut self, id: impl Into<String>) -> Self {
        self.domain_group_id = Some(id.into());
        self
    }

    pub fn approves_to(mut self, email: impl Into<String>) -> Self {
        self.approves_to = Some(email.into());
        self
    }

    pub fn role(mut self, role: PolicyRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn additional_policy_ids<I>(mut self, ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        self.additional_policy_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn remove_from_unassigned_policies(mut self) -> Self {
        self.remove_from_unassigned_policies = true;
        self
    }

    pub fn default_tags<I>(mut self, tags: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.default_tags = tags.into_iter().map(Into::into).collect();
        self
    }
}

/// Where Expensify gets the employee feed (`dataSource`).
#[derive(Clone, Debug)]
pub enum EmployeeSource {
    /// Feed sent inline in the request (`dataSource: "request"`).
    Inline(Vec<Employee>),
    /// Expensify downloads the feed (`dataSource: "download"`).
    FetchUrl {
        url: String,
        user: Option<String>,
        password: Option<String>,
    },
    /// Expensify fetches the feed over SFTP (`dataSource: "sftp"`).
    Sftp {
        connection: SftpConnection,
        filename: String,
    },
}

/// `setEmployeePrimaryPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryPolicyMode {
    None,
    NewEmployees,
    AllEmployees,
}

/// Advanced Employee Updater (`type: "update"`, `inputSettings.type:
/// "employees"`, `entity: "generic"`).
#[must_use = "actions do nothing until awaited"]
pub struct UpdateEmployeesAction {
    client: Client,
    source: EmployeeSource,
    dry_run: bool,
    primary_policy: Option<PrimaryPolicyMode>,
    fix_approval_chains: bool,
    first_level_managers_only: bool,
    skip_notification_emails: bool,
    email_on_finish: Option<String>,
}

impl UpdateEmployeesAction {
    pub(crate) fn new(client: Client, source: EmployeeSource) -> Self {
        Self {
            client,
            source,
            dry_run: false,
            primary_policy: None,
            fix_approval_chains: true,
            first_level_managers_only: false,
            skip_notification_emails: false,
            email_on_finish: None,
        }
    }

    /// Report the diff without applying it.
    pub fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }

    pub fn primary_policy(mut self, mode: PrimaryPolicyMode) -> Self {
        self.primary_policy = Some(mode);
        self
    }

    /// Disable `shouldFixApprovalChains` (server default: on).
    pub fn no_approval_chain_fixes(mut self) -> Self {
        self.fix_approval_chains = false;
        self
    }

    pub fn first_level_managers_only(mut self) -> Self {
        self.first_level_managers_only = true;
        self
    }

    pub fn skip_notification_emails(mut self) -> Self {
        self.skip_notification_emails = true;
        self
    }

    /// Comma-separate multiple recipients.
    pub fn email_on_finish(mut self, recipients: impl Into<String>) -> Self {
        self.email_on_finish = Some(recipients.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct SkippedEmployee {
    pub email: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct EmployeeUpdateOutcome {
    /// Echoes whether this run was a dry run.
    pub dry_run: bool,
    pub updated_count: u64,
    /// Members that would be / were added, keyed by policy.
    pub added: HashMap<PolicyId, Vec<String>>,
    /// Members that would be / were removed, keyed by policy.
    pub removed: HashMap<PolicyId, Vec<String>>,
    /// Domain-group assignments, keyed by group ID.
    pub security_group_assignments: HashMap<String, Vec<String>>,
    pub skipped: Vec<SkippedEmployee>,
}

impl IntoFuture for UpdateEmployeesAction {
    type Output = Result<EmployeeUpdateOutcome, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}

/// Deprecated CSV employee updater. Multipart upload; superseded by the
/// Advanced Employee Updater ([`UpdateEmployeesAction`]).
#[cfg(feature = "employee-updater-deprecated")]
#[deprecated = "Expensify no longer maintains this job; use Client::update_employees"]
#[must_use = "actions do nothing until awaited"]
pub struct UpdateEmployeesCsvAction {
    client: Client,
    policy_id: PolicyId,
    csv: bytes::Bytes,
}

#[cfg(feature = "employee-updater-deprecated")]
#[allow(deprecated)]
impl UpdateEmployeesCsvAction {
    pub(crate) fn new(client: Client, policy_id: PolicyId, csv: bytes::Bytes) -> Self {
        Self { client, policy_id, csv }
    }
}

#[cfg(feature = "employee-updater-deprecated")]
#[allow(deprecated)]
impl IntoFuture for UpdateEmployeesCsvAction {
    /// The `nbEmployees` count.
    type Output = Result<u64, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}
