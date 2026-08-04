use std::fmt;
use std::sync::Arc;

use time::Date;

use crate::cards::DomainCardListAction;
use crate::employees::{EmployeeSource, UpdateEmployeesAction};
use crate::expense_rules::{CreateExpenseRuleAction, UpdateExpenseRuleAction};
use crate::expenses::{CreateExpensesAction, Expense};
use crate::export::{ExportReportsAction, ReportsQuery};
use crate::file::{DownloadAction, ExportedFile};
use crate::policy::{
    CreatePolicyAction, GetPoliciesBuilder, ListPoliciesAction, SetTagApproversAction,
    TagApprover, UpdatePolicyAction,
};
use crate::reconciliation::{ReconcileAction, ReconciliationScope};
use crate::reports::{
    CreateReportAction, ExpenseLine, ReimburseAction, ReimburseTargets, Strict,
};
use crate::template::{ExportTemplate, FromExport, ReconciliationTemplate};
use crate::types::{PolicyId, RuleId};

/// `partnerUserID` / `partnerUserSecret` pair generated at
/// expensify.com/tools/integrations/.
#[derive(Clone)]
pub struct Credentials {
    partner_user_id: String,
    partner_user_secret: String,
}

impl Credentials {
    pub fn new(partner_user_id: impl Into<String>, partner_user_secret: impl Into<String>) -> Self {
        Self {
            partner_user_id: partner_user_id.into(),
            partner_user_secret: partner_user_secret.into(),
        }
    }
}

/// Manual impl: the secret must never reach logs.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("partner_user_id", &self.partner_user_id)
            .field("partner_user_secret", &"<redacted>")
            .finish()
    }
}

pub(crate) struct RateGate {
    per_10s: governor::DefaultDirectRateLimiter,
    per_60s: governor::DefaultDirectRateLimiter,
}

struct ClientInner {
    http: reqwest::Client,
    credentials: Credentials,
    base_url: reqwest::Url,
    limiter: Option<RateGate>,
}

/// Entry point. Cheaply cloneable (`Arc` internally); actions hold a
/// clone, so a `Client` need not outlive the actions it creates.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

impl Client {
    /// Default configuration: production endpoint, built-in rate limiting.
    pub fn new(credentials: Credentials) -> Self {
        Self::builder(credentials).build()
    }

    pub fn builder(credentials: Credentials) -> ClientBuilder {
        ClientBuilder {
            credentials,
            base_url: None,
            http: None,
            rate_limiting: true,
        }
    }

    // ---- exports ----------------------------------------------------

    /// Report Exporter. Resolves to an [`ExportedFile`] handle; pass it to
    /// [`Client::download`] to fetch the rendered output.
    pub fn export_reports<F>(
        &self,
        template: &ExportTemplate<F>,
        query: ReportsQuery,
    ) -> ExportReportsAction<F> {
        ExportReportsAction::new(self.clone(), template, query)
    }

    /// Downloader. The file handle carries both the correct `fileSystem`
    /// and the template's output type.
    pub fn download<F: FromExport>(&self, file: &ExportedFile<F>) -> DownloadAction<F> {
        DownloadAction::new(self.clone(), file)
    }

    // ---- policies ---------------------------------------------------

    pub fn list_policies(&self) -> ListPoliciesAction {
        ListPoliciesAction::new(self.clone())
    }

    /// Policy Getter. Select at least one field (`with_categories()`,
    /// `with_tax()`, ...) before awaiting; the selection shapes the
    /// returned [`crate::Policy`] at the type level.
    pub fn get_policies<I>(&self, policy_ids: I) -> GetPoliciesBuilder
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        GetPoliciesBuilder::new(self.clone(), policy_ids.into_iter().map(Into::into).collect())
    }

    pub fn create_policy(&self, name: impl Into<String>) -> CreatePolicyAction {
        CreatePolicyAction::new(self.clone(), name.into())
    }

    pub fn update_policy(&self, policy_id: impl Into<PolicyId>) -> UpdatePolicyAction {
        UpdatePolicyAction::new(self.clone(), vec![policy_id.into()])
    }

    /// Apply one update across several policies (`policyIDList`).
    pub fn update_policies<I>(&self, policy_ids: I) -> UpdatePolicyAction
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        UpdatePolicyAction::new(self.clone(), policy_ids.into_iter().map(Into::into).collect())
    }

    pub fn set_tag_approvers<I>(
        &self,
        policy_id: impl Into<PolicyId>,
        approvers: I,
    ) -> SetTagApproversAction
    where
        I: IntoIterator<Item = TagApprover>,
    {
        SetTagApproversAction::new(
            self.clone(),
            policy_id.into(),
            approvers.into_iter().collect(),
        )
    }

    // ---- reports & expenses -----------------------------------------

    pub fn create_report<I>(
        &self,
        policy_id: impl Into<PolicyId>,
        employee_email: impl Into<String>,
        title: impl Into<String>,
        expenses: I,
    ) -> CreateReportAction
    where
        I: IntoIterator<Item = ExpenseLine>,
    {
        CreateReportAction::new(
            self.clone(),
            policy_id.into(),
            employee_email.into(),
            title.into(),
            expenses.into_iter().collect(),
        )
    }

    pub fn create_expenses<I>(&self, expenses: I) -> CreateExpensesAction
    where
        I: IntoIterator<Item = Expense>,
    {
        CreateExpensesAction::new(self.clone(), expenses.into_iter().collect())
    }

    /// The only report-status transition Expensify supports: Approved →
    /// Reimbursed.
    pub fn mark_reports_reimbursed(&self, targets: ReimburseTargets) -> ReimburseAction<Strict> {
        ReimburseAction::new(self.clone(), targets)
    }

    // ---- expense rules ----------------------------------------------

    pub fn create_expense_rule(
        &self,
        policy_id: impl Into<PolicyId>,
        employee_email: impl Into<String>,
    ) -> CreateExpenseRuleAction {
        CreateExpenseRuleAction::new(self.clone(), policy_id.into(), employee_email.into())
    }

    pub fn update_expense_rule(
        &self,
        policy_id: impl Into<PolicyId>,
        employee_email: impl Into<String>,
        rule_id: RuleId,
    ) -> UpdateExpenseRuleAction {
        UpdateExpenseRuleAction::new(
            self.clone(),
            policy_id.into(),
            employee_email.into(),
            rule_id,
        )
    }

    // ---- employees --------------------------------------------------

    /// Advanced Employee Updater.
    pub fn update_employees(&self, source: EmployeeSource) -> UpdateEmployeesAction {
        UpdateEmployeesAction::new(self.clone(), source)
    }

    /// Deprecated CSV employee updater (multipart upload).
    #[cfg(feature = "employee-updater-deprecated")]
    #[allow(deprecated)]
    #[deprecated = "Expensify no longer maintains this job; use Client::update_employees"]
    pub fn update_employees_csv(
        &self,
        policy_id: impl Into<PolicyId>,
        csv: bytes::Bytes,
    ) -> crate::employees::UpdateEmployeesCsvAction {
        crate::employees::UpdateEmployeesCsvAction::new(self.clone(), policy_id.into(), csv)
    }

    // ---- domain scope -----------------------------------------------

    /// Scope to a domain for domain-level operations (reconciliation,
    /// card list). The domain name is data these jobs require, not a
    /// capability claim: Expensify still checks that the credentials are
    /// a domain admin.
    pub fn domain(&self, domain: impl Into<String>) -> DomainClient {
        DomainClient { client: self.clone(), domain: domain.into() }
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("credentials", &self.inner.credentials)
            .finish()
    }
}

pub struct ClientBuilder {
    credentials: Credentials,
    base_url: Option<reqwest::Url>,
    http: Option<reqwest::Client>,
    rate_limiting: bool,
}

impl ClientBuilder {
    /// Override the endpoint (testing / proxies).
    pub fn base_url(mut self, url: reqwest::Url) -> Self {
        self.base_url = Some(url);
        self
    }

    /// Bring your own `reqwest::Client` (proxies, timeouts, TLS config).
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Disable the built-in 5-per-10s / 20-per-60s limiter, e.g. when an
    /// external limiter governs the credentials.
    pub fn no_rate_limiting(mut self) -> Self {
        self.rate_limiting = false;
        self
    }

    pub fn build(self) -> Client {
        todo!()
    }
}

/// A [`Client`] scoped to one domain.
#[derive(Clone, Debug)]
pub struct DomainClient {
    client: Client,
    domain: String,
}

impl DomainClient {
    pub fn name(&self) -> &str {
        &self.domain
    }

    /// Reconciliation job. Resolves to an [`ExportedFile`] on the
    /// `reconciliation` file system (set here, not by the caller).
    pub fn reconcile<F>(
        &self,
        template: &ReconciliationTemplate<F>,
        start: Date,
        end: Date,
        scope: ReconciliationScope,
    ) -> ReconcileAction<F> {
        ReconcileAction::new(
            self.client.clone(),
            self.domain.clone(),
            template,
            start,
            end,
            scope,
        )
    }

    /// Domain Cards Getter.
    pub fn card_list(&self) -> DomainCardListAction {
        DomainCardListAction::new(self.client.clone(), self.domain.clone())
    }
}
