//! Misuse 17: setting an `onFinish` message on an action that has no message
//! field. Expensify carries `message` only on the email action, so on any
//! other one it would be accepted here and dropped on the wire.

use expensify::OnFinish;

fn main() {
    let _ = OnFinish::mark_as_exported("acme-etl").message("month end close");
}
