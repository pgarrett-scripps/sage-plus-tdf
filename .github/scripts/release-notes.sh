#!/usr/bin/env bash
# Print the CHANGELOG.md section for one version, for use as GitHub release notes.
#
#   .github/scripts/release-notes.sh 0.2.0 [CHANGELOG.md]
#
# The section starts at a heading "## <version>" or "## <version>, <date>" and
# ends at the next "## " heading. Fails when the section is missing, empty or
# still marked unreleased.
set -euo pipefail

version="${1:?usage: release-notes.sh <version> [changelog]}"
changelog="${2:-CHANGELOG.md}"

heading="$(awk -v ver="$version" '
    /^## / {
        rest = substr($0, 4)
        if (rest == ver || index(rest, ver ",") == 1 || index(rest, ver " ") == 1) { print; exit }
    }' "$changelog")"

if [ -z "$heading" ]; then
    echo "error: no '## $version' section in $changelog" >&2
    exit 1
fi
if printf '%s' "$heading" | grep -qi 'unreleased'; then
    echo "error: $changelog still marks $version as unreleased: $heading" >&2
    exit 1
fi

notes="$(awk -v heading="$heading" '
    $0 == heading { found = 1; next }
    found && /^## / { exit }
    found { print }' "$changelog" | sed -e '/./,$!d')"

if [ -z "${notes//[[:space:]]/}" ]; then
    echo "error: the $version section of $changelog is empty" >&2
    exit 1
fi
printf '%s\n' "$notes"
