#!/usr/bin/env bash
# Point a Homebrew formula's url/sha256 pairs at a new release.
#
# The formula carries no `version` stanza — brew scans the version out of the
# url and `brew audit --strict` rejects the redundancy — so the url is the only
# way a version reaches it. Digests are matched to urls by target triple rather
# than by position, so no target can inherit another target's sha256.
#
# Usage: bump-tap-formula.sh <formula> <version> <tag> <digests>
#   <digests> is `shasum -a 256` output naming the release archives, one line
#   per url in the formula.
set -euo pipefail

if [ "$#" -ne 4 ]; then
    echo "usage: bump-tap-formula.sh <formula> <version> <tag> <digests>" >&2
    exit 1
fi

formula="$1"
version="$2"
tag="$3"
digests="$4"

die() {
    echo "bump-tap-formula: $*" >&2
    exit 1
}

names=()
shas=()
triples=()
used=()
while read -r sha name; do
    [ -n "${sha:-}" ] && [ -n "${name:-}" ] || continue
    case "$sha" in
    *[!0-9a-f]*) die "'$sha' is not a hex digest" ;;
    esac
    [ "${#sha}" -eq 64 ] || die "'$sha' is not 64 hex digits"
    case "$name" in
    *.tar.gz) ;;
    *) die "'$name' is not a .tar.gz" ;;
    esac
    base="${name%.tar.gz}"
    triple="${base#*"-${version}-"}"
    [ "$triple" != "$base" ] || die "'$name' does not name version ${version}"
    names+=("$name")
    shas+=("$sha")
    triples+=("$triple")
    used+=("")
done <"$digests"

[ "${#names[@]}" -gt 0 ] || die "no digests in ${digests}"

out="$(mktemp)"
trap 'rm -f "$out"' EXIT

# -1 when the last url has had its sha256 rewritten, else its digest index.
pending=-1
lineno=0
while IFS= read -r line || [ -n "$line" ]; do
    lineno=$((lineno + 1))

    if [[ "$line" =~ ^([[:space:]]*)url[[:space:]]+\"([^\"]*)\"[[:space:]]*$ ]]; then
        [ "$pending" -lt 0 ] || die "line ${lineno}: a second url before the first one's sha256"
        indent="${BASH_REMATCH[1]}"
        url="${BASH_REMATCH[2]}"

        idx=-1
        for i in "${!names[@]}"; do
            case "$url" in
            *"-${triples[$i]}.tar.gz")
                [ "$idx" -lt 0 ] || die "line ${lineno}: url matches ${triples[$idx]} and ${triples[$i]}"
                idx="$i"
                ;;
            esac
        done
        [ "$idx" -ge 0 ] || die "line ${lineno}: the release has no archive for ${url}"
        [ -z "${used[$idx]}" ] || die "line ${lineno}: ${triples[$idx]} has more than one url"
        used[idx]=1

        origin="${url%%/releases/download/*}"
        [ "$origin" != "$url" ] || die "line ${lineno}: not a release download url: ${url}"
        printf '%surl "%s"\n' "$indent" "${origin}/releases/download/${tag}/${names[$idx]}" >>"$out"
        pending="$idx"
        continue
    fi

    if [ "$pending" -ge 0 ]; then
        [[ "$line" =~ ^([[:space:]]*)sha256[[:space:]]+\"[^\"]*\"[[:space:]]*$ ]] ||
            die "line ${lineno}: expected a sha256 after the url, got: ${line}"
        printf '%ssha256 "%s"\n' "${BASH_REMATCH[1]}" "${shas[$pending]}" >>"$out"
        pending=-1
        continue
    fi

    printf '%s\n' "$line" >>"$out"
done <"$formula"

[ "$pending" -lt 0 ] || die "the formula's last url has no sha256"

for i in "${!names[@]}"; do
    # A target the release builds but the formula does not serve. Adding one
    # means writing a new on_macos/on_linux branch, which is a judgement call.
    [ -n "${used[$i]}" ] || die "${names[$i]} has no url in the formula; add the target by hand"
done

cat "$out" >"$formula"
echo "bump-tap-formula: ${formula} -> ${tag} (${#names[@]} targets)"
