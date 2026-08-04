use time::{Date, PrimitiveDateTime};

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::wire;

/// One card from the Domain Cards Getter.
#[derive(Clone, Debug)]
pub struct DomainCard {
    /// Issuing bank / feed name.
    pub bank: String,
    /// Expensify's card identifier.
    pub card_id: i64,
    /// Card nickname.
    pub card_name: String,
    /// Masked, e.g. `1234XXXXXXXX1979`.
    pub card_number: String,
    /// Email of the assigned cardholder.
    pub email: String,
    /// External employee number, when the domain provides one.
    pub external_employee_id: Option<String>,
    /// When the card was added.
    pub created: Option<PrimitiveDateTime>,
    /// Last successful feed import.
    pub last_import: Option<PrimitiveDateTime>,
    /// responseCode of the last feed import.
    pub last_import_result: Option<u16>,
    /// Whether transactions on this card default to reimbursable.
    pub reimbursable: bool,
    /// Earliest date the feed scrapes from. Empty string on the wire
    /// becomes `None`.
    pub scrape_min_date: Option<Date>,
}

/// Domain Cards Getter (`type: "get"`, `inputSettings.type:
/// "domainCardList"`). Domain-admin credentials required
/// (server-enforced). No options.
#[must_use = "actions do nothing until awaited"]
pub struct DomainCardListAction {
    client: Client,
    domain: String,
}

impl DomainCardListAction {
    pub(crate) fn new(client: Client, domain: String) -> Self {
        Self { client, domain }
    }
}

impl IntoFuture for DomainCardListAction {
    type Output = Result<Vec<DomainCard>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::card_list(&self.domain);
            let response = self.client.send(request).await?;
            wire::domain_cards(response)
        })
    }
}
