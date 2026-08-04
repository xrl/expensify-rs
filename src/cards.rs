use time::{Date, PrimitiveDateTime};

use crate::client::Client;
use crate::error::Error;
use crate::BoxFuture;

/// One card from the Domain Cards Getter.
#[derive(Clone, Debug)]
pub struct DomainCard {
    pub bank: String,
    pub card_id: i64,
    pub card_name: String,
    /// Masked, e.g. `1234XXXXXXXX1979`.
    pub card_number: String,
    pub email: String,
    pub external_employee_id: Option<String>,
    pub created: Option<PrimitiveDateTime>,
    pub last_import: Option<PrimitiveDateTime>,
    /// responseCode of the last feed import.
    pub last_import_result: Option<u16>,
    pub reimbursable: bool,
    /// Empty string on the wire becomes `None`.
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
            let _ = self;
            todo!()
        })
    }
}
