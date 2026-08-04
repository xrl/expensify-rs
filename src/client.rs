use std::fmt;
use std::sync::Arc;

use time::Date;

use crate::Url;
use crate::cards::DomainCardListAction;
use crate::employees::{EmployeeSource, UpdateEmployeesAction};
use crate::expense_rules::{CreateExpenseRuleAction, UpdateExpenseRuleAction};
use crate::expenses::{CreateExpensesAction, Expense};
use crate::export::{ExportReportsAction, ReportsQuery};
use crate::file::{DownloadAction, ExportedFile};
use crate::policy::{
    CreatePolicyAction, GetPoliciesBuilder, GetPoliciesDynamicAction, ListPoliciesAction,
    PolicyField, SetTagApproversAction, TagApprover, UpdatePolicyAction,
};
use crate::reconciliation::{ReconcileAction, ReconciliationScope};
use crate::reports::{CreateReportAction, ExpenseLine, ReimburseAction, ReimburseTargets, Strict};
use crate::template::{ExportTemplate, FromExport, ReconciliationTemplate};
use crate::types::{PolicyId, RuleId};

/// The single production endpoint every job posts to.
pub(crate) const DEFAULT_ENDPOINT: &str =
    "https://integrations.expensify.com/Integration-Server/ExpensifyIntegrations";

/// `partnerUserID` / `partnerUserSecret` pair generated at
/// expensify.com/tools/integrations/.
///
/// The secret is shown once by Expensify and is never echoed by this type:
/// the [`fmt::Debug`] impl redacts it.
#[derive(Clone)]
pub struct Credentials {
    pub(crate) partner_user_id: String,
    pub(crate) partner_user_secret: String,
}

impl Credentials {
    /// Wrap an ID/secret pair.
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

/// Replace a URL's `user:pass@` with `<redacted>@`, keeping scheme, host and
/// path — the parts a printed URL is printed for.
///
/// Hand-rolled rather than via `Url`: the inputs are caller-supplied strings
/// that may not parse, and `url::Url`'s own `Debug` prints the password.
pub(crate) fn redact_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority = scheme_end + 3;
    let authority_end = url[authority..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |offset| authority + offset);
    match url[authority..authority_end].rfind('@') {
        None => url.to_owned(),
        Some(at) => {
            let mut redacted = String::with_capacity(url.len());
            redacted.push_str(&url[..authority]);
            redacted.push_str("<redacted>@");
            redacted.push_str(&url[authority + at + 1..]);
            redacted
        }
    }
}

pub(crate) struct RateGate {
    pub(crate) per_10s: governor::DefaultDirectRateLimiter,
    pub(crate) per_60s: governor::DefaultDirectRateLimiter,
}

pub(crate) struct ClientInner {
    pub(crate) http: reqwest::Client,
    pub(crate) credentials: Credentials,
    pub(crate) base_url: Url,
    pub(crate) limiter: Option<RateGate>,
}

/// Entry point. Cheaply cloneable (`Arc` internally); actions hold a
/// clone, so a `Client` need not outlive the actions it creates.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<ClientInner>,
}

impl Client {
    /// Default configuration: production endpoint, built-in rate limiting.
    pub fn new(credentials: Credentials) -> Self {
        Self::builder(credentials).build()
    }

    /// Start configuring a client (custom endpoint, HTTP client, or no
    /// rate limiting).
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

    /// Policy List Getter: every policy the credentials can see.
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
        GetPoliciesBuilder::new(
            self.clone(),
            policy_ids.into_iter().map(Into::into).collect(),
        )
    }

    /// Policy Getter with a **runtime** field selection — the escape hatch
    /// from the typestate, for callers whose selection is data.
    ///
    /// [`Client::get_policies`] is the default and stays the recommendation:
    /// it returns a [`Policy`](crate::Policy) whose sections are plain
    /// fields, so there is nothing to unwrap and reading a section you did
    /// not request does not compile. This method cannot offer that — the
    /// selection is not known until run time — so every section of the
    /// returned [`DynamicPolicy`](crate::DynamicPolicy) is an `Option`, and
    /// the `unwrap` the typestate exists to eliminate comes back.
    ///
    /// Reach for it only when the field list genuinely arrives as data (CLI
    /// flags, a config file, an RPC): writing the 32-way branch over `with_*`
    /// by hand is worse than this. When the selection *is* in the source, use
    /// [`Client::get_policies`].
    ///
    /// An empty `fields` list is rejected with
    /// [`Error::InvalidRequest`](crate::Error::InvalidRequest) at `.await` —
    /// the same rule the static path enforces at compile time by making
    /// [`GetPoliciesBuilder`] unawaitable. Repeated fields are ignored.
    ///
    /// ```no_run
    /// # async fn f(client: &expensify::Client, fields: Vec<expensify::PolicyField>)
    /// #     -> Result<(), expensify::Error> {
    /// let policies = client.get_policies_dynamic(["0123456789ABCDEF"], &fields).await?;
    /// for (id, policy) in &policies {
    ///     if let Some(categories) = &policy.categories {
    ///         println!("{id}: {} categories", categories.len());
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_policies_dynamic<I, F>(&self, policy_ids: I, fields: F) -> GetPoliciesDynamicAction
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
        F: IntoIterator,
        F::Item: Into<PolicyField>,
    {
        GetPoliciesDynamicAction::new(
            self.clone(),
            policy_ids.into_iter().map(Into::into).collect(),
            fields.into_iter().map(Into::into).collect(),
        )
    }

    /// Policy Creator.
    pub fn create_policy(&self, name: impl Into<String>) -> CreatePolicyAction {
        CreatePolicyAction::new(self.clone(), name.into())
    }

    /// Policy Updater for one policy. Requires policy-admin credentials
    /// (Expensify answers 403 otherwise).
    pub fn update_policy(&self, policy_id: impl Into<PolicyId>) -> UpdatePolicyAction {
        UpdatePolicyAction::new(self.clone(), vec![policy_id.into()])
    }

    /// Apply one update across several policies (`policyIDList`).
    pub fn update_policies<I>(&self, policy_ids: I) -> UpdatePolicyAction
    where
        I: IntoIterator,
        I::Item: Into<PolicyId>,
    {
        UpdatePolicyAction::new(
            self.clone(),
            policy_ids.into_iter().map(Into::into).collect(),
        )
    }

    /// Tag Approvers Updater. Requires policy-admin credentials and a
    /// single-level tag list; both are server-enforced.
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

    /// Report Creator.
    ///
    /// Restricted: Expensify support must enable report creation for your
    /// domain, and the credentials need both domain-admin and policy-admin
    /// rights. Without that the job fails with "Not authorized to
    /// authenticate as user" as an [`Error::Api`](crate::Error::Api).
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

    /// Expense Creator. Expenses land in the credential owner's account
    /// unless [`CreateExpensesAction::employee_email`] says otherwise.
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

    /// Expense Rules Creator. Set at least one action
    /// ([`CreateExpenseRuleAction::tag`] or
    /// [`CreateExpenseRuleAction::default_billable`]).
    pub fn create_expense_rule(
        &self,
        policy_id: impl Into<PolicyId>,
        employee_email: impl Into<String>,
    ) -> CreateExpenseRuleAction {
        CreateExpenseRuleAction::new(self.clone(), policy_id.into(), employee_email.into())
    }

    /// Expense Rules Updater.
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

    /// Advanced Employee Updater. Requires domain-admin credentials
    /// (server-enforced).
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
        DomainClient {
            client: self.clone(),
            domain: domain.into(),
        }
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            // `Url`'s own Debug prints `password` verbatim, and a caller-set
            // `base_url` may carry one.
            .field("base_url", &redact_userinfo(self.inner.base_url.as_str()))
            .field("credentials", &self.inner.credentials)
            .finish()
    }
}

/// Configures a [`Client`]. Obtain one from [`Client::builder`].
#[must_use = "builders do nothing until `build()` is called"]
pub struct ClientBuilder {
    credentials: Credentials,
    base_url: Option<Url>,
    http: Option<reqwest::Client>,
    rate_limiting: bool,
}

impl ClientBuilder {
    /// Override the endpoint (testing / proxies).
    ///
    /// Spell the argument as [`expensify::Url`](crate::Url) — a re-export, so
    /// this needs no `url` or `reqwest` dependency of your own:
    ///
    /// ```
    /// # fn f() -> Result<(), Box<dyn std::error::Error>> {
    /// use expensify::{Client, Credentials, Url};
    ///
    /// let client = Client::builder(Credentials::new("id", "secret"))
    ///     .base_url(Url::parse("http://127.0.0.1:8080/expensify")?)
    ///     .build();
    /// # let _ = client;
    /// # Ok(())
    /// # }
    /// ```
    pub fn base_url(mut self, url: Url) -> Self {
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

    /// Finish configuration.
    ///
    /// # Panics
    ///
    /// Only if the compiled-in default endpoint fails to parse, which
    /// cannot happen for a caller-visible reason.
    pub fn build(self) -> Client {
        let base_url = self.base_url.unwrap_or_else(|| {
            DEFAULT_ENDPOINT
                .parse()
                .expect("compiled-in endpoint is a valid URL")
        });
        Client {
            inner: Arc::new(ClientInner {
                http: self.http.unwrap_or_default(),
                credentials: self.credentials,
                base_url,
                limiter: self.rate_limiting.then(RateGate::new),
            }),
        }
    }
}

/// A [`Client`] scoped to one domain.
#[derive(Clone, Debug)]
pub struct DomainClient {
    client: Client,
    domain: String,
}

impl DomainClient {
    /// The domain this client is scoped to.
    pub fn name(&self) -> &str {
        &self.domain
    }

    /// Reconciliation job. Resolves to an [`ExportedFile`] on the
    /// `reconciliation` file system (set here, not by the caller).
    ///
    /// Requires domain-admin credentials for this domain; Expensify
    /// answers 403 otherwise.
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

    /// Domain Cards Getter. Requires domain-admin credentials.
    pub fn card_list(&self) -> DomainCardListAction {
        DomainCardListAction::new(self.client.clone(), self.domain.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_secret() {
        let creds = Credentials::new("partner-id", "hunter2-super-secret");
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
        assert!(rendered.contains("partner-id"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn client_debug_redacts_the_secret() {
        let client = Client::new(Credentials::new("partner-id", "hunter2-super-secret"));
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
    }

    #[test]
    fn client_debug_redacts_base_url_userinfo() {
        let client = Client::builder(Credentials::new("partner-id", "s3cret"))
            .base_url(
                "https://proxy:hunter2-super-secret@gw.acme.com/expensify"
                    .parse()
                    .unwrap(),
            )
            .build();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("hunter2-super-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>@gw.acme.com"), "{rendered}");
        assert!(rendered.contains("/expensify"), "{rendered}");
    }

    #[test]
    fn redaction_keeps_the_debuggable_half() {
        assert_eq!(
            redact_userinfo("https://hr:pw@acme.com/feed.json?x=1"),
            "https://<redacted>@acme.com/feed.json?x=1"
        );
        // An `@` in the path is not userinfo.
        assert_eq!(
            redact_userinfo("https://acme.com/feed@latest.json"),
            "https://acme.com/feed@latest.json"
        );
        assert_eq!(redact_userinfo("not a url"), "not a url");
    }
}
