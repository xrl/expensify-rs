//! Misuse 15: a third `FetchState`. Only "requested" and "not requested"
//! have meaning, and a third state would have no sound `extract`, so the
//! trait is sealed.

use expensify::FetchState;

struct Maybe;

impl FetchState for Maybe {
    type Wrap<T: expensify::Payload> = T;

    // Deliberately complete: the seal must be what rejects this impl, not a
    // missing item.
    fn project<T: expensify::Payload>(wrapped: T) -> Option<T> {
        Some(wrapped)
    }

    fn extract<T>(
        _field: &'static str,
        _value: Option<serde_json::Value>,
    ) -> Result<T, expensify::Error>
    where
        T: serde::de::DeserializeOwned + expensify::Payload,
    {
        unimplemented!()
    }
}

fn main() {}
