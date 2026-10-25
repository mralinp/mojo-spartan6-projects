#!/usr/bin/env bash
# Self-check for next-version.sh and changelog.sh in a throwaway repo.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
git init -q -b main && git config user.email t@t && git config user.name t
c() { git commit -q --allow-empty -m "$1"; }
eq() { [ "$1" = "$2" ] || { echo "FAIL: got '$1', want '$2'"; exit 1; }; }

c "chore: init";                         eq "$("$here/next-version.sh")" 0.1.0
git tag v0.1.0;                          eq "$("$here/next-version.sh")" 0.1.0   # already released
c "docs: tweak (#2)";                    eq "$("$here/next-version.sh")" 0.1.1
c "feat(net): scan subnets (#3)";        eq "$("$here/next-version.sh")" 0.2.0
c "fix: ssh timeout (#4)";               eq "$("$here/next-version.sh")" 0.2.0
c "feat!: drop python (#5)";             eq "$("$here/next-version.sh")" 1.0.0
notes=$("$here/changelog.sh" 1.0.0 o/r)
grep -q "Breaking changes" <<<"$notes"
grep -q "\*\*net:\*\* scan subnets (\[#3\](https://github.com/o/r/pull/3))" <<<"$notes"
grep -q "ssh timeout" <<<"$notes"
grep -q "compare/v0.1.0...v1.0.0" <<<"$notes"
echo "release scripts ok"
