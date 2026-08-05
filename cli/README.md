# expensify-cli

Command-line client for the [Expensify Integration Server API](https://integrations.expensify.com/Integration-Server/doc/),
built on the [`expensify`](https://crates.io/crates/expensify) library.

```console
$ cargo install expensify-cli
$ expensify auth login
$ expensify get policies
```

The binary is named `expensify`.

## Commands

Verb-noun, like `kubectl`. `-o json` on every read command, and exit codes
documented in `expensify --help` so scripts can branch on them.

```console
$ expensify auth login|status|logout
$ expensify get policies|policy|cards
$ expensify export reports|reconciliation
$ expensify download <HANDLE>
$ expensify create policy|expenses|report|expense-rule
$ expensify update policy|tag-approvers|expense-rule|employees
$ expensify reimburse
$ expensify completion <SHELL>
$ expensify skill install
```

## Seeing the wire

`-v` logs one line per API call — job type, endpoint, status, size, timing.
`-vv` adds the whole exchange: the request body as sent with credentials
redacted, and the response body verbatim with its content-type, which is what
you need when Expensify answers something other than the documented envelope.
`-vvv` adds transport tracing from the HTTP stack.

**`-vv` prints response bodies as they arrived**, and those routinely contain
employee names, email addresses and masked card numbers. It names the account
whose data it is about to print, because that is what decides whether the log
can be published. Read a log before pasting it into a ticket.

## When something fails

A failure prints the account it authenticated as, and — where the client cannot
account for what happened — a fingerprint of the failure's shape:

```
account: aa_you_example_com (from OS keychain)
defect fingerprint: EXP-9CAE0FE8  [export.reports exit=10 decode.json]
```

The fingerprint is derived from the command, the exit code and the error's
discriminant, and from nothing that moves between releases, so the same defect
answers the same token every time. Search issues for it exactly before filing:

```console
$ gh issue list --repo xrl/expensify-rs --search EXP-9CAE0FE8 --state all
```

Failures the client can account for — permissions, not found, rejected, rate
limited, missing credentials, network — are not defects and carry no
fingerprint.

## Credentials

Generate a partner pair at <https://www.expensify.com/tools/integrations/>.
Expensify shows the secret **exactly once**.

Resolution order is flags → environment (`EXPENSIFY_PARTNER_USER_ID` /
`EXPENSIFY_PARTNER_USER_SECRET`) → OS keychain. A source supplying one half of
the pair and not the other is an error rather than a fall-through, so a stale
keychain entry can't silently pair with a fresh environment variable. Use the
environment variables in CI — a runner has no keychain and `auth login` needs a
TTY.

Keychain access is granted per executable, so a binary the OS has not seen —
**every `cargo build` produces one** — raises a permission prompt on its first
read, and nothing outside an interactive session can answer it. The read is
bounded accordingly: 10s with no terminal attached, 120s with one, then exit 3
naming the ways out. `EXPENSIFY_KEYCHAIN_TIMEOUT_SECS` overrides the limit and
`0` waits indefinitely.

## Agent skill

`expensify skill install` writes a [Claude Code](https://claude.com/claude-code)
skill into your skills directory. It covers what `--help` cannot: which
operations need permissions you must request from Expensify support, which are
irreversible, and which wire behaviours are unverified. `--print` writes it to
stdout instead if you want to read it first.

## Status

The wire format is **only partly verified against a live Expensify account** —
Expensify publishes no OpenAPI spec, so most field names and value types are
derived from their prose documentation. A dozen response shapes have been
recorded live and five documented claims were wrong; the library's
`docs/DESIGN.md` § Verification status says which is which. Some operations are
deliberately withheld rather than shipped half-known; `expensify <command>
--help` says which and why.

Treat a surprising response as a plausible client bug rather than user error,
and please report it.

## License

MIT OR Apache-2.0, at your option.
