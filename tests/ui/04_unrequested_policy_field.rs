//! Misuse 4: reading a policy section that was never requested. `categories`
//! holds `NotFetched`, which has no data and no methods.

use expensify::{Client, Credentials, Error, PolicyId};

async fn run(client: &Client, id: PolicyId) -> Result<(), Error> {
    let policies = client.get_policies([&id]).with_tax().await?;

    for _category in &policies[&id].categories {}
    Ok(())
}

fn main() {
    let _ = run(
        &Client::new(Credentials::new("id", "secret")),
        PolicyId::new("P1"),
    );
}
