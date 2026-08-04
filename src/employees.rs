//! Advanced Employee Updater (documented at `doc/employeeUpdater/`), plus
//! the deprecated CSV employee updater behind the
//! `employee-updater-deprecated` feature.

use std::collections::HashMap;
use std::fmt;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::export::SftpConnection;
use crate::policy::PolicyRole;
use crate::types::PolicyId;
use crate::wire;

/// One employee record in the feed. Required fields in the constructor.
#[derive(Clone, Debug)]
pub struct Employee {
    pub(crate) employee_email: String,
    pub(crate) manager_email: String,
    pub(crate) employee_id: String,
    pub(crate) policy_id: PolicyId,
    pub(crate) first_name: Option<String>,
    pub(crate) last_name: Option<String>,
    pub(crate) custom_field_1: Option<String>,
    pub(crate) custom_field_2: Option<String>,
    pub(crate) approval_limit: Option<i64>,
    pub(crate) over_limit_approver: Option<String>,
    pub(crate) worker_status: Option<String>,
    pub(crate) is_terminated: Option<bool>,
    pub(crate) domain_group_id: Option<String>,
    pub(crate) approves_to: Option<String>,
    pub(crate) role: Option<PolicyRole>,
    pub(crate) additional_policy_ids: Vec<PolicyId>,
    pub(crate) remove_from_unassigned_policies: bool,
    pub(crate) default_tags: Vec<String>,
}

impl Employee {
    /// `employee_id` drives email-change detection: Expensify matches on
    /// it and merges accounts when the email differs. It also auto-fills
    /// Custom Field 1 unless [`Employee::custom_field_1`] overrides it.
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

    /// Given name.
    pub fn first_name(mut self, name: impl Into<String>) -> Self {
        self.first_name = Some(name.into());
        self
    }

    /// Family name.
    pub fn last_name(mut self, name: impl Into<String>) -> Self {
        self.last_name = Some(name.into());
        self
    }

    /// Custom Field 1; overrides the auto-fill from `employee_id`.
    pub fn custom_field_1(mut self, value: impl Into<String>) -> Self {
        self.custom_field_1 = Some(value.into());
        self
    }

    /// Custom Field 2.
    pub fn custom_field_2(mut self, value: impl Into<String>) -> Self {
        self.custom_field_2 = Some(value.into());
        self
    }

    /// Amount this person may approve without escalation.
    pub fn approval_limit(mut self, limit: i64) -> Self {
        self.approval_limit = Some(limit);
        self
    }

    /// Who approves reports above [`Employee::approval_limit`].
    pub fn over_limit_approver(mut self, email: impl Into<String>) -> Self {
        self.over_limit_approver = Some(email.into());
        self
    }

    /// Free-text; "On Leave" removes the person from manager duty.
    pub fn worker_status(mut self, status: impl Into<String>) -> Self {
        self.worker_status = Some(status.into());
        self
    }

    /// Mark the employee as departed.
    pub fn terminated(mut self) -> Self {
        self.is_terminated = Some(true);
        self
    }

    /// Domain group to assign. Expensify only applies group assignment when
    /// *every* record in the feed carries one.
    pub fn domain_group_id(mut self, id: impl Into<String>) -> Self {
        self.domain_group_id = Some(id.into());
        self
    }

    /// Next approver in the chain above this person.
    pub fn approves_to(mut self, email: impl Into<String>) -> Self {
        self.approves_to = Some(email.into());
        self
    }

    /// Role on the primary policy.
    pub fn role(mut self, role: PolicyRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Further policies to add this person to.
    pub fn additional_policy_ids<I>(mut self, ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        self.additional_policy_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Remove the person from any policy not named in this record.
    pub fn remove_from_unassigned_policies(mut self) -> Self {
        self.remove_from_unassigned_policies = true;
        self
    }

    /// Tags applied to the person's new expenses by default.
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
///
/// [`fmt::Debug`] redacts the feed password — and any `user:pass@` in the
/// feed URL — as [`Credentials`](crate::Credentials) and [`SftpConnection`]
/// do.
#[derive(Clone)]
pub enum EmployeeSource {
    /// Feed sent inline in the request (`dataSource: "request"`).
    Inline(Vec<Employee>),
    /// Expensify downloads the feed (`dataSource: "download"`).
    FetchUrl {
        /// Where to fetch the JSON feed from.
        url: String,
        /// Basic-auth user, if the URL needs one.
        user: Option<String>,
        /// Basic-auth password. Never printed by [`fmt::Debug`].
        password: Option<String>,
    },
    /// Expensify fetches the feed over SFTP (`dataSource: "sftp"`).
    Sftp {
        /// Server to connect to.
        connection: SftpConnection,
        /// Feed filename relative to the SFTP user's home directory.
        filename: String,
    },
}

impl fmt::Debug for EmployeeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline(employees) => f.debug_tuple("Inline").field(employees).finish(),
            Self::FetchUrl {
                url,
                user,
                password,
            } => f
                .debug_struct("FetchUrl")
                // `https://user:pass@host/feed.json` is a natural way to spell
                // this, so the URL is a secret carrier like the rest.
                .field("url", &crate::client::redact_userinfo(url))
                .field("user", user)
                .field("password", &password.as_ref().map(|_| "<redacted>"))
                .finish(),
            Self::Sftp {
                connection,
                filename,
            } => f
                .debug_struct("Sftp")
                .field("connection", connection)
                .field("filename", filename)
                .finish(),
        }
    }
}

/// `setEmployeePrimaryPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryPolicyMode {
    /// Leave primary policies alone (server default).
    None,
    /// Set the primary policy only for newly added employees.
    NewEmployees,
    /// Set the primary policy for every employee in the feed.
    AllEmployees,
}

/// Advanced Employee Updater (`type: "update"`, `inputSettings.type:
/// "employees"`, `entity: "generic"`). Requires domain-admin credentials.
#[must_use = "actions do nothing until awaited"]
pub struct UpdateEmployeesAction {
    pub(crate) client: Client,
    pub(crate) source: EmployeeSource,
    pub(crate) dry_run: bool,
    pub(crate) primary_policy: Option<PrimaryPolicyMode>,
    pub(crate) fix_approval_chains: bool,
    pub(crate) first_level_managers_only: bool,
    pub(crate) skip_notification_emails: bool,
    pub(crate) email_on_finish: Option<String>,
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

    /// Whether and for whom to set the primary policy.
    pub fn primary_policy(mut self, mode: PrimaryPolicyMode) -> Self {
        self.primary_policy = Some(mode);
        self
    }

    /// Disable `shouldFixApprovalChains` (server default: on).
    pub fn no_approval_chain_fixes(mut self) -> Self {
        self.fix_approval_chains = false;
        self
    }

    /// Only repair the first level of each approval chain. Has no effect
    /// alongside [`UpdateEmployeesAction::no_approval_chain_fixes`].
    pub fn first_level_managers_only(mut self) -> Self {
        self.first_level_managers_only = true;
        self
    }

    /// Do not email employees about the changes.
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

/// An employee record the updater declined to apply.
#[derive(Clone, Debug)]
pub struct SkippedEmployee {
    /// Email from the feed record.
    pub email: String,
    /// Expensify's explanation.
    pub reason: String,
}

/// What the Advanced Employee Updater did, or would have done under
/// [`UpdateEmployeesAction::dry_run`].
#[derive(Clone, Debug)]
pub struct EmployeeUpdateOutcome {
    /// Echoes whether this run was a dry run.
    pub dry_run: bool,
    /// `updatedEmployeesCount`.
    pub updated_count: u64,
    /// Members that would be / were added, keyed by policy.
    pub added: HashMap<PolicyId, Vec<String>>,
    /// Members that would be / were removed, keyed by policy.
    pub removed: HashMap<PolicyId, Vec<String>>,
    /// Domain-group assignments, keyed by group ID.
    pub security_group_assignments: HashMap<String, Vec<String>>,
    /// Records that were not applied.
    pub skipped: Vec<SkippedEmployee>,
}

impl IntoFuture for UpdateEmployeesAction {
    type Output = Result<EmployeeUpdateOutcome, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::update_employees(&self);
            let response = self.client.send(request).await?;
            wire::employee_outcome(response)
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
        Self {
            client,
            policy_id,
            csv,
        }
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
            let request = wire::update_employees_csv(&self.policy_id, self.csv);
            let response = self.client.send(request).await?;
            wire::nb_employees(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_feed_password() {
        let source = EmployeeSource::FetchUrl {
            url: "https://hr.acme.com/feed.json".into(),
            user: Some("hr".into()),
            password: Some("hunter2-super-secret".into()),
        };
        let rendered = format!("{source:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains("hr.acme.com"), "{rendered}");

        // No password, nothing to hide.
        let anonymous = EmployeeSource::FetchUrl {
            url: "https://hr.acme.com/feed.json".into(),
            user: None,
            password: None,
        };
        assert!(!format!("{anonymous:?}").contains("<redacted>"));
    }

    #[test]
    fn debug_redacts_userinfo_in_the_feed_url() {
        let source = EmployeeSource::FetchUrl {
            url: "https://hr:hunter2-super-secret@hr.acme.com/feed.json".into(),
            user: None,
            password: None,
        };
        let rendered = format!("{source:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>@hr.acme.com"), "{rendered}");
        assert!(rendered.contains("/feed.json"), "{rendered}");
    }

    #[test]
    fn debug_redacts_the_sftp_feed_password() {
        let source = EmployeeSource::Sftp {
            connection: SftpConnection {
                host: "sftp.acme.com".into(),
                login: "acme".into(),
                password: "hunter2-super-secret".into(),
                port: 22,
            },
            filename: "employees.json".into(),
        };
        let rendered = format!("{source:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
        assert!(rendered.contains("employees.json"), "{rendered}");
    }
}
