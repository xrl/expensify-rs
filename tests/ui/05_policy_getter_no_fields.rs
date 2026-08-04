//! Misuse 5: awaiting the policy getter without selecting a field. Expensify
//! answers 410; here the builder simply is not a future.

use expensify::{Client, Credentials, Error};

async fn run(client: &Client) -> Result<(), Error> {
    let _policies = client.get_policies(["P1"]).await?;
    Ok(())
}

fn main() {
    let _ = run(&Client::new(Credentials::new("id", "secret")));
}
