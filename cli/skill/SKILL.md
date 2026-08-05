---
name: expensify
description: >
  Drive the `expensify` CLI against the Expensify Integration Server API: partner
  credentials, policies (categories, tags, report fields, tax rates, members),
  company cards, FreeMarker report exports and card reconciliation, expense and
  report creation, expense rules, the employee updater, and marking approved reports
  reimbursed. Use when the user mentions Expensify, expense reports, a partner user
  ID/secret, policy IDs, exporting or reconciling expense data, or reimbursing
  reports. Carries what `--help` cannot: where credentials come from, which
  operations need domain-admin rights or an unlock from Expensify support, which
  are irreversible, and which wire behaviours are unverified guesses.
---

# expensify CLI

One binary over Expensify's single-endpoint Integration Server API. Verb-noun
commands (`expensify get policies`, `expensify export reports`), `-o json` on
every read, exit codes listed in `expensify --help`.

**Most of the wire format is derived from Expensify's prose docs, not from a
live account** — there is no OpenAPI spec. Eleven response shapes have now been
confirmed against a real account (2026-08-04) and five documented claims turned
out to be wrong; `docs/DESIGN.md` § Verification status says which is which.
Anything still marked inferred is a guess, so treat a surprising response as a
plausible client bug, not as user error.

## Before you run anything

### Irreversible or costly

| Command | What it really does |
|---|---|
| `export reports --mark-as-exported LABEL` | Permanently labels the exported reports. `--not-exported-as LABEL` then skips them on every later run. This API has no unmark. |
| `reimburse` | A real financial state change, Approved → Reimbursed. It is the only transition Expensify offers; there is no way back. |
| `update policy --tags` / `--tags-csv` | **Replaces** every tag on the policy. Tags not in the file are deleted — confirmed live, silently and under a 200. |
| `update policy --categories-mode replace-all`, `--report-fields-mode replace-all` | Deletes every category / report field not in the file. Default is `merge`; only ask for `replace-all` deliberately. |
| `export reports --email`, `export reconciliation --email-on-finish` | Actually sends mail to real people. |
| `update employees` | Moves people between policies and rewrites approval chains. `--dry-run` first — it reports what would change without changing it. |

`--test-run` on `export reports` is **not** a trustworthy dry run. The encoding
of Expensify's `test` flag is inferred from a parameter table and unconfirmed;
if the guess is wrong the flag is a no-op and every on-finish action fires
anyway, including `--mark-as-exported` and `--email`. Make an export safe by
leaving the destructive flags off, not by adding `--test-run`.

### Safe to explore with

`auth status`, `get policies`, `get policy`, `get cards`, `download`,
`completion`, `skill install --print`, and `export reports` / `export
reconciliation` with no `--mark-as-exported` and no `--email*`. All read-only
or file-producing.

## Credentials

Generate a partner pair at <https://www.expensify.com/tools/integrations/>.
**Expensify shows the secret exactly once** — it cannot be read back, only
regenerated.

Resolution order. A source that supplies one half of the pair and not the other
is an error, not a fall-through, so a stale keychain entry can never silently
pair with a fresh environment variable:

1. `--partner-user-id` / `--partner-user-secret`
2. `EXPENSIFY_PARTNER_USER_ID` / `EXPENSIFY_PARTNER_USER_SECRET`
3. OS keychain — `expensify auth login`, secret read without echo, never
   written to a file

`expensify auth status` reports which source won and the secret's *length*,
never the secret. In CI use the environment variables: a runner has no keychain
and `auth login` needs a TTY.

## Permissions you cannot grant yourself

| Needs | Commands |
|---|---|
| policy admin | `get policy --with-employees` / `--with-tax`, `update policy`, `update tag-approvers` |
| domain admin | `get cards`, `export reconciliation`, `update employees` |
| possibly an unlock from Expensify support | `create report`; `--on-behalf-of` anywhere (third-party access grant) |

`create report` is documented as needing that unlock and domain-admin rights,
and worked on a policy-admin account with neither — so the requirement is
unconfirmed rather than absent. `create expenses --employee-email` is likewise
documented as restricted, and creating expenses for the credential owner's own
address needed no grant.

Exit 4 is a flat refusal — check the credential pair's account roles. A
*server* error (exit 1) on `create report` or `--on-behalf-of` usually means
"support has not enabled this for your account" rather than "Expensify is
down"; Expensify reuses its server-error code for that case.

## Export is two-step, and only half of it is synchronous

`export` starts a job and prints a **file handle**; `download` fetches it.

- `export reports` renders **asynchronously**. The handle arrives before the
  file exists and Expensify publishes no ready signal. Downloading too early
  fails with exit 10 and `download returned an empty body; the export may not
  have finished rendering` — that message is the retry signal. Back off and
  retry the *download*; re-running the export burns another job.
- `export reconciliation` runs **synchronously**. Its handle is immediately
  downloadable.

The two write to different stores. `download` defaults to `--file-system
integration-server`; a reconciliation file needs `--file-system reconciliation`
or you get a 404 or garbage. Copy both columns out of the export's output
instead of retyping the filename.

Every export needs `--template`: a FreeMarker source file, or `-` for stdin.
There is no default template and no built-in report shape.

`--mark-as-exported` fires when the export *job* succeeds, not when you have
the bytes. If rendering or the download then fails, the reports are labelled
anyway and `--not-exported-as` will skip them forever. For data you cannot
re-derive, export unlabelled, confirm the download, then re-export exactly
those `--report-id`s with the label.

## Worked example: export JSON, download, act on the rows

```console
$ cat > reports.ftl <<'EOF'
[<#list reports as report>
  {"report_id": "${report.reportID}",
   "employee": "${report.accountEmail}",
   "total_cents": ${report.total}}<#if report_has_next>,</#if>
</#list>]
EOF

$ expensify export reports \
    --template reports.ftl \
    --since 2026-07-01 --until 2026-08-01 \
    --policy-id 1234ABCD \
    --state approved \
    --not-exported-as acme-etl \
    --format json \
    -o json
{
  "file_system": "integration-server",
  "filename": "export_8f3d.json"
}

$ expensify download export_8f3d.json -O july.json     # exit 10 → sleep, retry
$ jq -r '.[] | "\(.report_id) \(.total_cents)"' july.json
```

`--format` defaults to `csv` for *every* template, including one that emits
JSON. Set it explicitly or the file's bytes will not match its extension.

`download` writes raw bytes to stdout and ignores `-o`; use `-O PATH` for
anything binary (`xls`, `xlsx`).

## `-o json` shapes

`table` (default) and `wide` are for humans; parse `json`. An empty table
result prints `No <noun> found.` on **stderr** with nothing on stdout, so
`| wc -l` stays honest; `-o json` gives `[]`.

```jsonc
// get policies — array
[{"id":"1234ABCD","name":"Engineering","owner":"cfo@acme.com",
  "role":"admin","output_currency":"USD","plan":"corporate"}]

// get policy — array, one object per policy ID, SECTIONS YOU DID NOT ASK FOR
// ARE ABSENT (not null). `"tax": null` means the policy has no tax config.
[{"id":"1234ABCD",
  "categories":[{"name":"Meals","enabled":true,"gl_code":"4000","payroll_code":null,
                 "comment_hint":null,"are_comments_required":false,
                 "max_expense_amount_cents":null}],
  "tags":{"shape":"flat","tags":[{"name":"Core","enabled":true,"gl_code":null}]},
  "tax":{"name":"VAT","default_rate_id":"id_A",
         "rates":[{"name":"Standard","rate":20.0,"rate_id":"id_A"}]}}]

// export reports | export reconciliation
{"filename":"export_8f3d.csv","file_system":"integration-server"}

// reimburse
{"updated":["R1","R2"]}
// reimburse --tolerate-partial
{"updated":["R1"],"skipped":[{"report_id":"R2","reason":"not approved"}],"failed":[]}

// create expenses — array. `report_id` is the report Expensify put the
// expense in, which it opens for you if the expense named none.
[{"transaction_id":"T1","report_id":"R009WqAY45L1","merchant":"Cloud Hosting Inc",
  "date":"2026-07-31","amount_cents":12900,"currency":"USD"}]

// writes Expensify answers with no body (create expense-rule, update policy, ...)
{"result":"created a rule for a@acme.com on 1234ABCD"}
```

`get policy --with-tags` answers in two shapes because Expensify does:
`{"shape":"flat","tags":[…]}` or `{"shape":"levels","levels":[{"name":…,"tags":[…]}]}`.
Handle both.

## Input files

Anything too big for flags is a JSON array read from a path or `-` (stdin),
**`snake_case` throughout** — unlike the wire, which is `camelCase`. Unknown
keys are rejected, not ignored, so a typo is a loud parse error.

```jsonc
// create expenses --file   (merchant, date, amount_cents required; currency defaults USD)
[{"merchant":"Cloud Hosting Inc","date":"2026-07-31","amount_cents":12900,
  "category":"Infrastructure","external_id":"hosting-2026-07",
  "tax":{"rate_id":"id_A","amount_cents":2150}}]

// create report --expenses   (only these four fields exist here)
[{"merchant":"Taxi","date":"2026-07-14","amount_cents":4200,"currency":"USD"}]

// update policy --categories
[{"name":"Meals","enabled":true,"gl_code":"4000","require_comments":true}]

// update policy --report-fields   (type: text | dropdown | date — no formula)
[{"name":"Cost Center","type":"dropdown","values":["Ops",{"name":"Eng","enabled":false}]}]

// update policy --tags   (REPLACES every tag on the policy)
[{"name":"Department","required":true,"tags":[{"name":"Eng","gl_code":"100"}]}]

// update employees --file
[{"employee_email":"a@acme.com","manager_email":"b@acme.com",
  "employee_id":"E1","policy_id":"1234ABCD"}]
```

Amounts are always **minor units as integers**: `12900` is $129.00. Dates are
strictly `YYYY-MM-DD` — `07/01/2026` is a usage error.

## Gotchas worth knowing before you hit them

- `get policy` needs at least one `--with-*`; Expensify rejects a request that
  names no fields, and each flag costs a section of response.
- `--until` only narrows `--since` / `--approved-after`. With `--report-id` it
  is a usage error (exit 2), not a silent no-op.
- Date windows may not exceed a year, and an end date becomes *required* once
  the start anchor is over a year old.
- `create expense-rule` returns no rule ID — the live response is only
  `{"responseMessage":"OK","responseCode":200}`. `update expense-rule
  --rule-id N` therefore needs an ID from somewhere else, and the one known
  source is an accident: re-creating an identical rule answers the
  undocumented `responseCode 666`, `Rule already exists with those actions,
  please update rule N`. Plan for that before creating rules you intend to edit.
- `create expenses` requires `--employee-email`. It does not default to the
  credential owner; without it Expensify answers 410.
- `create expenses` without `--report-id` does not leave the expense loose:
  Expensify opens a report for it. The `report_id` column names that report,
  and it is the only way to find the expense without a separate export.
- `reimburse` treats a partially applied run as an error (exit 8) — including
  the case Expensify reports as a plain `responseCode: 200` with every report
  skipped, which it does. Add `--tolerate-partial` to get the
  `updated`/`skipped`/`failed` breakdown as data instead; read both lists.
- Rate limits are 5 requests / 10 s and 20 / 60 s. The CLI paces itself
  automatically; `--no-rate-limit` opts out. The limiter is per-process, so
  several processes sharing one credential pair still need pacing of their own.
  A 429 is exit 7 with no auto-retry.
- Exit 10 ("unreadable response") is the one to re-run with `-vv`: it prints
  the request as sent, credentials redacted, and the raw response body with
  its content-type, which is normally enough to see what Expensify actually
  answered. Response bodies contain employee names and email addresses — do
  not paste that output anywhere without reading it first.
- Useful exit codes for branching: `2` usage, `3` no credentials, `4`
  permission denied, `5` not found, `6` rejected as invalid, `7` rate limited,
  `8` partial success, `9` network, `10` unreadable response (including the
  not-yet-rendered download).

## Deliberately not offered

Not oversights — each returns as an additive change once someone with a live
account confirms the behaviour:

- **PDF export.** `--format` has no `pdf`. Expensify emits one PDF *per report*
  and a single file handle cannot name several files, so a PDF export would
  silently hand back a fraction of the data.
- **Tag merging.** `update policy --tags*` replaces only, and this one is not
  coming back: a tags update sent with `action: "merge"` was observed deleting
  every unlisted tag and answering `{"responseCode":200}` with no warning.
- **SFTP and URL employee feeds.** The library supports both; the CLI does not,
  because the password would sit in `argv` where any `ps` can read it. Use
  `--file` or stdin.

## Install this skill elsewhere

```console
$ expensify skill install              # ~/.claude/skills/expensify/SKILL.md
$ expensify skill install --project    # ./.claude/skills/expensify/SKILL.md
$ expensify skill install --print      # to stdout, install nothing
```

It refuses to overwrite an existing file unless given `--force`.
