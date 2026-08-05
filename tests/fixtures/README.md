# Recorded responses

Bodies captured from the live Integration Server on 2026-08-04 through
`ClientBuilder::observe` + `Recorder`, and replayed by `tests/replay.rs`
through a mock server. Status and content-type live in that file's `Fixture`
table, since they are two scalars each and belong beside the assertion.

Add one by installing a `Recorder`, running the real call once, and writing
`exchange.body()` here verbatim — not by writing down what the response
*should* be. Four of the five defects these fixtures cover were shipped
because a hand-authored mock asserted this crate's own inference back at it.

`create-expenses.json` is the one exception worth flagging: the transaction
object is verbatim, the `responseCode`/`transactionList` envelope around it is
the shape every other job uses and was not re-captured separately.
