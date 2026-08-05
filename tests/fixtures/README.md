# Recorded responses

Bodies captured from the live Integration Server on 2026-08-04 through
`ClientBuilder::observe` + `Recorder`, and replayed by `tests/replay.rs`
through a mock server. Status and content-type live in that file's `Fixture`
table, since they are two scalars each and belong beside the assertion.

Add one by installing a `Recorder`, running the real call once, and writing
`exchange.body()` here verbatim — not by writing down what the response
*should* be, and not by back-translating the CLI's snake_case rendering into
camelCase, which is the same mistake wearing a disguise. Four of the five
defects these fixtures cover were shipped because a hand-authored mock
asserted this crate's own inference back at it.

Every body here is a raw response. `create-expenses.json` carries four fields
this crate does not model (`comment`, `tag`, `mcc`, `category`) — leave them
in: they are what makes the replay a test of unknown-field tolerance.
