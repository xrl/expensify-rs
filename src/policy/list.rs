use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::policy::model::PolicySummary;
use crate::wire;

/// Policy List Getter (`type: "get"`, `inputSettings.type: "policyList"`).
#[must_use = "actions do nothing until awaited"]
pub struct ListPoliciesAction {
    pub(crate) client: Client,
    pub(crate) admin_only: bool,
    pub(crate) user_email: Option<String>,
}

impl ListPoliciesAction {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            admin_only: false,
            user_email: None,
        }
    }

    /// Only policies where the user is an admin (`adminOnly: true`).
    pub fn admin_only(mut self) -> Self {
        self.admin_only = true;
        self
    }

    /// `userEmail`; requires a prior third-party access grant.
    pub fn on_behalf_of(mut self, email: impl Into<String>) -> Self {
        self.user_email = Some(email.into());
        self
    }
}

impl IntoFuture for ListPoliciesAction {
    type Output = Result<Vec<PolicySummary>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::list_policies(&self);
            let response = self.client.send(request).await?;
            wire::policy_list(response)
        })
    }
}
