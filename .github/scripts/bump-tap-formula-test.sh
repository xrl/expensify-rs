#!/usr/bin/env bash
# Self-test for bump-tap-formula.sh. Needs no network and no credentials.
#
# The fixtures are the real thing: the tap's formula before and after the 0.2.0
# release, and that release's published .sha256 sidecars. A pass means the
# script reproduces a bump a human already made and reviewed, byte for byte.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bump="${here}/bump-tap-formula.sh"
fixtures="${here}/fixtures"
digests="${fixtures}/expensify-cli-0.2.0.sha256"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cases=0
failures=0
ok() {
    cases=$((cases + 1))
    echo "ok   - $*"
}
bad() {
    cases=$((cases + 1))
    failures=$((failures + 1))
    echo "FAIL - $*" >&2
}

# check_bump <desc> <input> <expected> — succeeds, output matches byte for byte.
check_bump() {
    local desc="$1" input="$2" expected="$3"
    cp "$input" "${work}/f.rb"
    if ! "$bump" "${work}/f.rb" 0.2.0 expensify-cli-0.2.0 "$digests" >/dev/null; then
        bad "$desc: exited non-zero"
        return
    fi
    if cmp -s "${work}/f.rb" "$expected"; then
        ok "$desc"
    else
        bad "$desc"
        diff -u "$expected" "${work}/f.rb" >&2 || true
    fi
}

# check_rejects <desc> <expect> <input> [digests] — refuses for the stated
# reason, and leaves the formula alone. The reason is checked because a rewriter
# that refuses everything would otherwise pass every one of these.
check_rejects() {
    local desc="$1" expect="$2" input="$3" d="${4:-$digests}"
    cp "$input" "${work}/f.rb"
    if "$bump" "${work}/f.rb" 0.2.0 expensify-cli-0.2.0 "$d" >/dev/null 2>"${work}/err"; then
        bad "$desc: accepted it"
        return
    fi
    if ! cmp -s "${work}/f.rb" "$input"; then
        bad "$desc: refused, but wrote to the formula anyway"
        return
    fi
    if ! grep -qF "$expect" "${work}/err"; then
        bad "$desc: refused, but for another reason — $(head -1 "${work}/err")"
        return
    fi
    ok "$desc — $(head -1 "${work}/err")"
}

check_bump "0.1.0 -> 0.2.0 reproduces the tap's formula" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${fixtures}/expensify-cli-0.2.0.rb"

# Re-running a release must not churn the formula.
check_bump "re-bumping 0.2.0 is a no-op" \
    "${fixtures}/expensify-cli-0.2.0.rb" "${fixtures}/expensify-cli-0.2.0.rb"

# A target the release does not build must not silently keep its stale digest.
sed 's/x86_64-unknown-linux-gnu/x86_64-apple-darwin/g' \
    "${fixtures}/expensify-cli-0.1.0.rb" >"${work}/unserved.rb"
check_rejects "a url the release has no archive for" \
    "the release has no archive for" "${work}/unserved.rb"

# The url/sha256 adjacency is the whole pairing rule; if it does not hold, stop.
sed '/sha256/d' "${fixtures}/expensify-cli-0.1.0.rb" >"${work}/no-sha.rb"
check_rejects "a url with no sha256 under it" \
    "expected a sha256 after the url" "${work}/no-sha.rb"

# The release grew a target the formula has no block for.
cp "$digests" "${work}/extra.sha256"
printf '%s  expensify-cli-0.2.0-x86_64-apple-darwin.tar.gz\n' \
    "$(printf 'c%.0s' $(seq 64))" >>"${work}/extra.sha256"
check_rejects "an archive with no url" "has no url in the formula" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/extra.sha256"

# Truncated or otherwise mangled sidecar.
sed 's/^4223/zzzz/' "$digests" >"${work}/bad.sha256"
check_rejects "a digest that is not hex" "is not a hex digest" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/bad.sha256"

# A sidecar from a different release must not be pasted into this one.
sed 's/0\.2\.0/0.3.0/g' "$digests" >"${work}/mismatched.sha256"
check_rejects "an archive naming another version" "does not name version" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/mismatched.sha256"

if [ "$failures" -eq 0 ]; then
    echo "bump-tap-formula-test: ${cases} cases, the rewriter reproduces its fixtures"
    exit 0
fi

# Name the thing that broke: this runs in a Rust matrix and again mid-release,
# where a bare non-zero exit from a shell script explains nothing.
summary="the tap formula rewriter no longer reproduces its fixtures (${failures} of ${cases} cases failed)"
[ -z "${GITHUB_ACTIONS:-}" ] || echo "::error title=Tap formula rewriter::${summary}"
echo "bump-tap-formula-test: ${summary}" >&2
exit 1
