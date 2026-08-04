use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::types::{PolicyId, RuleId};
use crate::wire;

/// Expense Rules Creator (`type: "create"`, `inputSettings.type:
/// "expenseRules"`). At least one action ([`tag`](Self::tag) /
/// [`default_billable`](Self::default_billable)) must be set; Expensify
/// validates at runtime with a 410. Response shape is undocumented
/// upstream, so this resolves to `()` — note that this leaves no way to
/// learn the new [`RuleId`] needed by [`UpdateExpenseRuleAction`].
#[must_use = "actions do nothing until awaited"]
pub struct CreateExpenseRuleAction {
    pub(crate) client: Client,
    pub(crate) policy_id: PolicyId,
    pub(crate) employee_email: String,
    pub(crate) tag: Option<String>,
    pub(crate) default_billable: Option<bool>,
}

impl CreateExpenseRuleAction {
    pub(crate) fn new(client: Client, policy_id: PolicyId, employee_email: String) -> Self {
        Self {
            client,
            policy_id,
            employee_email,
            tag: None,
            default_billable: None,
        }
    }

    /// Auto-apply this tag to the employee's expenses.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Auto-set the billable flag on the employee's expenses.
    pub fn default_billable(mut self, billable: bool) -> Self {
        self.default_billable = Some(billable);
        self
    }
}

impl IntoFuture for CreateExpenseRuleAction {
    type Output = Result<(), Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::create_expense_rule(&self);
            self.client.send(request).await?;
            Ok(())
        })
    }
}

/// Expense Rules Updater (`type: "update"`). Identical to the creator
/// plus the `ruleID` of the rule to modify.
#[must_use = "actions do nothing until awaited"]
pub struct UpdateExpenseRuleAction {
    pub(crate) client: Client,
    pub(crate) policy_id: PolicyId,
    pub(crate) employee_email: String,
    pub(crate) rule_id: RuleId,
    pub(crate) tag: Option<String>,
    pub(crate) default_billable: Option<bool>,
}

impl UpdateExpenseRuleAction {
    pub(crate) fn new(
        client: Client,
        policy_id: PolicyId,
        employee_email: String,
        rule_id: RuleId,
    ) -> Self {
        Self {
            client,
            policy_id,
            employee_email,
            rule_id,
            tag: None,
            default_billable: None,
        }
    }

    /// Auto-apply this tag to the employee's expenses.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Auto-set the billable flag on the employee's expenses.
    pub fn default_billable(mut self, billable: bool) -> Self {
        self.default_billable = Some(billable);
        self
    }
}

impl IntoFuture for UpdateExpenseRuleAction {
    type Output = Result<(), Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::update_expense_rule(&self);
            self.client.send(request).await?;
            Ok(())
        })
    }
}
