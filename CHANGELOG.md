# Changelog

## Unreleased

- CLI: `expensify skill install` writes the Claude Code agent skill embedded in
  the binary (`cli/skill/SKILL.md`) into a personal or repository-local skills
  directory. The library is unchanged.

## 0.2.0 — 2026-08-04

Three additions for callers driving the API at run time rather than from
source. All additive; nothing in 0.1.0 changes shape.

- `FetchState::project` inverts the `Wrap` GAT — `Fetched` yields `Some`,
  `Omitted` yields `None` — so code generic over the fetch states can read a
  slot instead of re-deriving the trait locally. `Policy::project` applies it
  to all five sections at once.
- `Client::get_policies_dynamic(ids, fields)` takes the field selection as
  data (`PolicyField`) and answers `DynamicPolicy`, whose sections are
  `Option`s. This is an escape hatch, documented as one: it reintroduces the
  `unwrap` the typestate exists to remove, and `get_policies` remains the
  default. Both getters share one request path.
- `reqwest` is re-exported as `expensify::reqwest`, plus `expensify::Url`.
  Naming `ClientBuilder::base_url`'s argument — or `http_client`'s, or the
  types inside `Error::Transport` and `Error::Http` — no longer needs a
  second dependency.

## 0.1.0 — 2026-08-04

First release. Covers the Expensify Integration Server's export, download,
reconciliation, policy read/write, expense and report creation, reimbursement,
expense rules, tag approvers, and employee updater jobs.

Wire shapes are derived from Expensify's prose documentation and have not been
verified against a live account — see `docs/DESIGN.md` § Open questions, and the
rustdoc on individual methods where a guess carries consequences.

Deliberately withheld until a live probe confirms behavior; restoring each is
additive:

- Merging (rather than replacing) policy tags. Expensify's prose says a tags
  update replaces, and the inline parameter table documents no `action` key, so
  a method named `merge_*` could silently delete every unlisted tag.
- PDF export. Expensify emits one PDF per report, which a single `ExportedFile`
  handle cannot name.
