# Changelog

## 0.1.0 — unreleased

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
