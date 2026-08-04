use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::types::PolicyId;
use crate::wire;

/// One tag-approver assignment. Clearing is an explicit constructor, not
/// an empty-string sentinel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagApprover {
    pub(crate) name: String,
    /// `None` serializes as `""`, which Expensify treats as "clear".
    pub(crate) approver: Option<String>,
}

impl TagApprover {
    /// Route expenses tagged `tag_name` to `approver_email`.
    pub fn assign(tag_name: impl Into<String>, approver_email: impl Into<String>) -> Self {
        Self {
            name: tag_name.into(),
            approver: Some(approver_email.into()),
        }
    }

    /// Remove the approver from `tag_name`.
    pub fn clear(tag_name: impl Into<String>) -> Self {
        Self {
            name: tag_name.into(),
            approver: None,
        }
    }
}

/// Tag Approvers Updater (`type: "update"`, `inputSettings.type:
/// "tagApprovers"`). Policy-admin credentials and a single-level tag list
/// required (server-enforced).
#[must_use = "actions do nothing until awaited"]
pub struct SetTagApproversAction {
    pub(crate) client: Client,
    pub(crate) policy_id: PolicyId,
    pub(crate) approvers: Vec<TagApprover>,
}

impl SetTagApproversAction {
    pub(crate) fn new(client: Client, policy_id: PolicyId, approvers: Vec<TagApprover>) -> Self {
        Self {
            client,
            policy_id,
            approvers,
        }
    }
}

impl IntoFuture for SetTagApproversAction {
    type Output = Result<(), Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::set_tag_approvers(&self);
            self.client.send(request).await?;
            Ok(())
        })
    }
}
