# Prior art

The type design in [`DESIGN.md`](DESIGN.md) is adapted from Isabel Atkinson's
RustConf 2024 talk (Montreal) on implementing a language-agnostic specification
in Rust, which walks through the MongoDB Rust driver's then-new `bulkWrite` API.

## Why a MongoDB driver talk applies to an expense API

MongoDB drivers implement a written cross-language specification, and the
driver team's stated constraint is to "be as idiomatic as possible while
meeting the specification and staying true to the original intent." The talk is
a worked example of resolving that tension in Rust specifically.

Expensify's Integration Server sits in the same position with the tension
sharpened: a wire protocol designed for no particular language, consumed from
many. The difference is that we have no specification at all — no OpenAPI
document, no versioning, no changelog, no deprecation signal, just prose docs.
So the talk's method transfers directly while its safety net does not, which is
the one place we had to add machinery of our own (see
[Compliance testing](#compliance-testing-what-we-had-to-add)).

## The three takeaways, and where each landed

### 1. Continuity with the existing API

The talk's case: a generic on the write model would have infected the write
model enum and forced every operation in one `bulk_write` call to share a
single `T`, defeating the point of mixed writes. The fix was to move
construction onto `Collection<T>`, which already knew its own type and could
serialize at that boundary — so the new API borrowed the shape users already
knew from `insert_one`.

Applied here as a single shape for every one of the sixteen jobs:

```
client.verb_noun(required args) -> action struct -> fluent setters -> .await
```

The same "required args in the constructor, optionals as setters" rule governs
data builders, so `Expense::new(merchant, date, amount).category(..)` reads like
the operations do. One pattern learned once.

### 2. Options cost nothing when unused

The talk moves from a `typed_builder` options struct — construct it, set one
boolean, pass it in — to fluent setters chained directly onto the operation,
with `IntoFuture` supplying the `.await`. Users who set nothing think about
nothing.

Adopted wholesale: no options struct appears in any signature in this crate,
and no operation takes an `Option<Opts>` parameter.

The talk's sharper corollary got adopted too. `verbose_results` defaulted to
false, so the only reason to call it was to pass `true` — which made the
parameter dead weight. Deleting it turned the method into a statement of
intent. The same reasoning produces `.dry_run()`, `.test_run()`,
`.admin_only()`, and `.tolerate_partial()` here, and it also decides an
argument the talk never had to make: the reconciliation job documents an
`async` field whose only supported value is `false`, so no parameter exists to
set it.

### 3. Provide the best possible type information

The talk's central move: `verbose_results()` stops setting a boolean and
instead returns `BulkWriteAction<VerboseBulkWriteResult>`, with two
`IntoFuture` impls whose `Output` types differ. The compiler now knows which
result shape comes back, and the `unwrap()` on the verbose fields disappears
because they are no longer `Option`.

That move is applied three times in this crate, in ascending order of payoff:

**Templates, files, and downloads.** An export's response shape is defined by
the caller's FreeMarker template, so the library cannot type it alone. A
phantom parameter threads the caller's declared intent from
`ExportTemplate<Json<Vec<Row>>>` through `ExportedFile<Json<Vec<Row>>>` to
`download()`, which resolves to `Vec<Row>` with no annotation at the use site.

**Reimbursement.** Expensify returns `207` for partial success. The talk's move
applied to error strictness rather than result verbosity: the default
`IntoFuture` yields `Vec<ReportId>` and turns 207 into
`Err(Error::PartialSuccess)`, so there is no `skipped` list to forget to check;
`.tolerate_partial()` swaps in an impl yielding `ReimburseOutcome` where 207 is
`Ok`.

**The policy getter — `verbose_results` five times over.** The Policy Getter's
`fields` request list decides which response sections exist. Modeled at
runtime that is five `Option`s and an `unwrap` at every use site: precisely the
bug the talk fixed, multiplied. A sealed `FetchState` trait with a GAT
(`Fetched::Wrap<T> = T`, `Omitted::Wrap<T> = NotFetched`) makes
`Policy<Cats, Fields, Tags, Tax, Emps>` field-exact across all 32
combinations, served by one generic `IntoFuture` impl. `.with_tax()` is the
`verbose_results()` of this API.

The distinction that mechanism exists to draw shows up in the one `Option` we
kept: `tax: Tax::Wrap<Option<TaxConfig>>`. The outer wrapper is
request-dependent absence, the inner `Option` is data-dependent absence — a
policy that genuinely has no tax configuration. Collapsing those two into one
`Option` is the thing that forces the unwrap.

## Where we diverged

**Typestate is not the only tool, and reaching for it reflexively is the
failure mode.** The Downloader's `fileSystem` must match the job that produced
the filename, which reads like a textbook typestate problem. It turned out to
need no type parameter at all: `ExportedFile`'s fields are private, only
`export_reports` and `reconcile` can mint one, each baking in its own
`FileSystem`, and `download()` has no parameter to get wrong.
Non-constructibility was cheaper than typestate and produces a better compiler
error. Likewise "at least one export filter" is enforced by anchored
constructors rather than a builder typestate.

**Markers, not data types.** The talk's generic is instantiated at a real
result struct. Ours is instantiated at marker types that are never constructed,
because `FromExport::Output` has to differ from `Self` — `Json<Vec<Row>>`
decodes to `Vec<Row>`. Leaving that trait unsealed is what lets callers add
their own CSV or XML markers.

**A rule the talk did not need.** MongoDB's spec describes an API the driver
fully controls. Expensify has capability tiers the library cannot verify:
reconciliation needs domain-admin credentials, report creation needs
support-side enablement. A `assume_domain_admin()` phantom would encode an
unverified user assertion and change nothing about the runtime 403 — ceremony
without prevention. So this crate adds an explicit constraint on top of the
talk's three: *a phantom or sealed mechanism must make a statable misuse
unrepresentable*, and unverifiable facts stay runtime errors with documentation.
`DomainClient` survives only because the domain string is required job data.

## Method, not just result

The talk insists on writing the user example before finalizing any types, and
using it to pressure-test ergonomics as the design moves. `examples/tour.rs` is
that example — a month-end close carried through the whole design doc — and it
compiles in CI, so ergonomic regressions break the build rather than going
unnoticed.

## Compliance testing: what we had to add

MongoDB's specification ships with a cross-driver compliance test suite. There
is no equivalent here, and every claim the design makes about what *cannot* be
expressed is worthless if nothing checks it — the first person to loosen a
bound would silently undo it.

So the design's list of misuses is executable. Each of the twelve entries in
[`DESIGN.md` § Misuses made uncompilable](DESIGN.md#misuses-made-uncompilable)
is a `trybuild` case with a committed `.stderr`. That suite is this crate's
compliance test: it is the difference between "hard to hold wrong" as a design
intention and as a property under test.
