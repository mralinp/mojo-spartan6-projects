#!/usr/bin/env bash
# Prints the next semver (no "v") from Conventional Commits since the last v* tag.
# breaking (type!: or BREAKING CHANGE) -> major, feat -> minor, anything else -> patch. First release is 0.1.0.
set -euo pipefail
last=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
[ -n "$last" ] || { echo "0.1.0"; exit 0; }
if [ "$(git rev-list -n1 "$last")" = "$(git rev-parse HEAD)" ]; then echo "${last#v}"; exit 0; fi # already released
IFS=. read -r major minor patch <<<"${last#v}"
log=$(git log "$last"..HEAD --format='%s%n%b')
if grep -qE '^[a-z]+(\([^)]*\))?!:|^BREAKING[ -]CHANGE' <<<"$log"; then
  echo "$((major + 1)).0.0"
elif grep -qE '^feat(\([^)]*\))?:' <<<"$log"; then
  echo "${major}.$((minor + 1)).0"
else
  echo "${major}.${minor}.$((patch + 1))"
fi
