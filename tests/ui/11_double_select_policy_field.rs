//! Misuse 11: selecting the same policy field twice. `with_tax` exists only
//! on the impl block where that slot is still `Omitted`.

use expensify::{Client, Credentials};

fn main() {
    let client = Client::new(Credentials::new("id", "secret"));

    let _ = client.get_policies(["P1"]).with_tax().with_tax();
}
