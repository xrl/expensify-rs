//! Misuse 19: reaching a secret's contents without saying so. `Secret` is not
//! a smart pointer and does not deref — `expose()` is the only way in, which
//! is what makes every use of a secret greppable.

use expensify::Secret;

fn main() {
    let secret: Secret<String> = "hunter2".into();
    let leaked: &String = &*secret;
    println!("{leaked}");
}
