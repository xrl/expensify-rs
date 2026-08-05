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

failed=0
ok() { echo "ok   - $*"; }
bad() {
    echo "FAIL - $*" >&2
    failed=1
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

# check_rejects <desc> <input> [digests] — refuses, and leaves the formula alone.
check_rejects() {
    local desc="$1" input="$2" d="${3:-$digests}"
    cp "$input" "${work}/f.rb"
    if "$bump" "${work}/f.rb" 0.2.0 expensify-cli-0.2.0 "$d" >/dev/null 2>"${work}/err"; then
        bad "$desc: accepted it"
        return
    fi
    if ! cmp -s "${work}/f.rb" "$input"; then
        bad "$desc: refused, but wrote to the formula anyway"
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
check_rejects "a url the release has no archive for" "${work}/unserved.rb"

# The url/sha256 adjacency is the whole pairing rule; if it does not hold, stop.
sed '/sha256/d' "${fixtures}/expensify-cli-0.1.0.rb" >"${work}/no-sha.rb"
check_rejects "a url with no sha256 under it" "${work}/no-sha.rb"

# The release grew a target the formula has no block for.
cp "$digests" "${work}/extra.sha256"
printf '%s  expensify-cli-0.2.0-x86_64-apple-darwin.tar.gz\n' \
    "$(printf 'c%.0s' $(seq 64))" >>"${work}/extra.sha256"
check_rejects "an archive with no url" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/extra.sha256"

# Truncated or otherwise mangled sidecar.
sed 's/^4223/zzzz/' "$digests" >"${work}/bad.sha256"
check_rejects "a digest that is not hex" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/bad.sha256"

# A sidecar from a different release must not be pasted into this one.
sed 's/0\.2\.0/0.3.0/g' "$digests" >"${work}/mismatched.sha256"
check_rejects "an archive naming another version" \
    "${fixtures}/expensify-cli-0.1.0.rb" "${work}/mismatched.sha256"

exit "$failed"
