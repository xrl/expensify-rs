---
name: CLI defect
about: An `expensify` command failed, or answered something it should not have
title: 'expensify <command>: <symptom>'
labels: bug
---

<!--
Search before filing: gh issue list --repo xrl/expensify-rs --search "<command> <symptom>" --state all
Comment on an open issue rather than opening a near-duplicate.

Two things this repo cannot redact for you:
  - the command line below, if it carried --partner-user-secret VALUE
  - the transcript, if it came from an account with real people in it
This repository is public.
-->

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

**Account the transcript came from** (a disposable trial account, or one whose
identifying fields have been redacted):
