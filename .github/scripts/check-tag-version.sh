#!/usr/bin/env bash
# Refuse to release when the tag and the manifest disagree — the tag is already
# burned by the time anyone notices, and trusted publishing gives no second try.
#
# Usage: check-tag-version.sh <crate> <tag-prefix> [tag]   (tag defaults to $GITHUB_REF_NAME)
set -euo pipefail

crate="$1"
prefix="$2"
tag="${3:-${GITHUB_REF_NAME:?no tag given and GITHUB_REF_NAME is unset}}"

tagged="${tag#"$prefix"}"
if [ "$tagged" = "$tag" ] || [ -z "$tagged" ]; then
    echo "check-tag-version: tag '$tag' is not of the form '${prefix}<version>'" >&2
    exit 1
fi

manifest="$(cargo metadata --no-deps --format-version 1 --locked |
    jq -r --arg crate "$crate" '.packages[] | select(.name == $crate) | .version')"

if [ -z "$manifest" ]; then
    echo "check-tag-version: no crate named '$crate' in this workspace" >&2
    exit 1
fi

if [ "$tagged" != "$manifest" ]; then
    echo "check-tag-version: tag '$tag' releases $crate $tagged, but Cargo.toml says $manifest" >&2
    exit 1
fi

echo "check-tag-version: $crate $manifest matches tag '$tag'"
