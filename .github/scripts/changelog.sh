#!/usr/bin/env bash
# Usage: changelog.sh <version> <owner/repo>  -> release notes (markdown) on stdout, from commits since the last v* tag.
set -euo pipefail
version=$1 repo=$2
last=$(git describe --tags --abbrev=0 --match 'v[0-9]*' HEAD^ 2>/dev/null || git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
# if HEAD itself is the last tag (re-run), compare against the one before it
[ -z "$last" ] || [ "$(git rev-list -n1 "$last")" != "$(git rev-parse HEAD)" ] || last=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$last"^ 2>/dev/null || true)
range=${last:+$last..}HEAD
link() { sed -E "s|\(#([0-9]+)\)|([#\1](https://github.com/$repo/pull/\1))|g"; }

section() { # title, regex of types
  local out
  out=$(git log "$range" --no-merges --format='%s' | grep -E "^($2)(\([^)]*\))?!?: " | sed -E 's/^[a-z]+(\(([^)]*)\))?!?: /\1|/; s/^\(([^)]*)\)\|/**\1:** /; s/^\|//' | link || true)
  [ -z "$out" ] || { printf '### %s\n\n' "$1"; sed 's/^/- /' <<<"$out"; printf '\n'; }
}

breaking=$(git log "$range" --no-merges --format='%s%n%b' | grep -E '^[a-z]+(\([^)]*\))?!:|^BREAKING[ -]CHANGE' | link || true)
printf '## Mojo Spartan-6 Projects v%s\n\n' "$version"
[ -z "$breaking" ] || { printf '### ⚠ Breaking changes\n\n'; sed 's/^/- /' <<<"$breaking"; printf '\n'; }
section "Features" "feat"
section "Bug fixes" "fix"
section "Performance" "perf"
section "Refactoring" "refactor"
section "Documentation" "docs"
section "Build and CI" "build|ci"
section "Tests" "test"
section "Chores" "chore"
if [ -n "$last" ]; then printf '**Full changelog:** https://github.com/%s/compare/%s...v%s\n' "$repo" "$last" "$version"; fi
