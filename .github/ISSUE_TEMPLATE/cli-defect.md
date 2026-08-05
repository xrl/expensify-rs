---
name: CLI defect
about: An `expensify` command failed, or answered something it should not have
title: 'expensify <command>: <symptom> [EXP-XXXXXXXX]'
labels: bug
---

<!--
Search before filing, on the fingerprint rather than on words — two people
describe one defect two ways, and neither search finds the other:

  gh issue list --repo xrl/expensify-rs --search EXP-XXXXXXXX --state all

Comment on what it finds rather than opening a near-duplicate.

Two things this repo cannot redact for you:
  - the command line below, if it carried --partner-user-secret VALUE
  - the transcript, if it came from an account with real people in it
This repository is public.
-->

**Defect fingerprint** (the `defect fingerprint:` line on stderr, verbatim; say
so if the failure printed none):

```
```

**Version** (`expensify --version`):

**Command** (secret values replaced):

```console
$ expensify ...
```

**Exit code**:

**Expected**:

**Actual**:

**`-vv` transcript**, or why there is none — a `create`/`update`/`reimburse`, or
an export carrying `--mark-as-exported` or `--email*`, must not be re-run just
to collect one:

```
```

**Account the transcript came from** — the `account:` line the failure printed.
A disposable trial account may be pasted whole; anything else must have its
identifying fields redacted first:
