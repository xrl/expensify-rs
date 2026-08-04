use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::policy::model::PolicyPlan;
use crate::types::PolicyId;
use crate::wire;

/// Policy Creator (`type: "create"`, `inputSettings.type: "policy"`).
#[must_use = "actions do nothing until awaited"]
pub struct CreatePolicyAction {
    client: Client,
    name: String,
    plan: Option<PolicyPlan>,
}

impl CreatePolicyAction {
    pub(crate) fn new(client: Client, name: String) -> Self {
        Self {
            client,
            name,
            plan: None,
        }
    }

    /// Default: [`PolicyPlan::Team`] (server default).
    pub fn plan(mut self, plan: PolicyPlan) -> Self {
        self.plan = Some(plan);
        self
    }
}

/// Result of a successful Policy Creator run.
#[derive(Clone, Debug)]
pub struct CreatedPolicy {
    /// Identifier of the new policy.
    pub policy_id: PolicyId,
    /// Name as stored by Expensify.
    pub name: String,
}

impl IntoFuture for CreatePolicyAction {
    type Output = Result<CreatedPolicy, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::create_policy(&self.name, self.plan);
            let response = self.client.send(request).await?;
            wire::created_policy(response)
        })
    }
}
