# Releasing

Both crates release from a tag; the tag must match the version in `Cargo.toml`
or the run stops before publishing anything.

| crate | tag | workflow |
| --- | --- | --- |
| `expensify` | `expensify-<version>` | `release-lib.yml` |
| `expensify-cli` | `expensify-cli-<version>` | `release-cli.yml` |

Both publish to crates.io with trusted publishing, which is registered against
those exact workflow filenames — renaming or moving one makes that crate
unpublishable until the registration is redone.

`release-cli.yml` additionally creates a GitHub Release, uploads a `.tar.gz` and
`.tar.gz.sha256` for each of the three native targets, and then opens a pull
request against [`xrl/homebrew-tap`][tap] bumping `Formula/expensify-cli.rb`.

[tap]: https://github.com/xrl/homebrew-tap

## The tap credential

`GITHUB_TOKEN` is scoped to this repository, so it cannot write to the tap. The
`tap` job needs a repository secret named **`HOMEBREW_TAP_TOKEN`**:

- a **fine-grained personal access token**, owned by an account with write
  access to `xrl/homebrew-tap`
- **repository access:** only `xrl/homebrew-tap`
- **repository permissions:** `Contents: read and write`, `Pull requests: read
  and write` — nothing else
- stored in this repo under *Settings → Secrets and variables → Actions*

Fine-grained tokens expire. An expired one fails the job rather than skipping it
— the secret is still there, so the job cannot tell "not set up" from "set up
wrong", and only the first of those is worth passing over in silence. crates.io
and the release are untouched either way.

An App installation token would avoid the expiry and the personal ownership, but
costs an App registration, a private key to store and rotate, and a token-minting
step in the workflow. `repository_dispatch` into the tap would move the
credential problem rather than solve it: the tap would still need a token to
trigger, and the formula-rewriting logic would live away from the release that
produces the archives it describes.

## Without the credential

The `tap` job skips, loudly but green: a run annotation and a job summary saying
the tap was not updated. By the time it runs, crates.io and the GitHub Release
have already succeeded, and a red run for a downstream nicety only teaches people
to ignore red runs. Bump the formula by hand, or set the secret and re-run the
job — it is idempotent.

## Reviewing the tap PR

The PR lists the three digests and links the release. It is opened, never pushed
to the tap's `main`, because a bad formula breaks `brew install` for everyone and
this path runs unattended. Before merging, confirm the digests match the release
assets, and that `brew audit --strict --online expensify-cli` and `brew install
--build-from-source` are clean — the tap has no CI of its own.

The formula deliberately carries no `version` stanza; brew scans the version out
of the url, and `brew audit --strict` rejects having both.

## Re-running

Re-running a release is safe. The tap branch is named for the version, so a rerun
reuses it rather than opening a second PR, and produces an identical formula —
no commit, no push, no force-push. If the tap already serves the version, the job
says so and stops.
