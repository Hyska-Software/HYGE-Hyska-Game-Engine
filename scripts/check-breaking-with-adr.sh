#!/usr/bin/env bash
# scripts/check-breaking-with-adr.sh
#
# Enforces the "every breaking change requires an ADR" invariant
# (AGENTS.md §11, docs/ownership.md §9 "Architecture impact").
#
# Fails the CI job if, in the range of commits since the last release tag
# (or since the root commit if no tag exists), there is at least one
# breaking change but no new file was added under docs/adr/.
#
# A "breaking change" is detected by git-cliff's conventional-commit parser
# in cliff.toml: subject has a "!" suffix (e.g. "feat!:" or "fix!:") or
# the body contains a "BREAKING CHANGE:" footer.
#
# This script is intentionally self-contained and uses only POSIX + git + awk
# so it works on any GitHub Actions runner (Linux, Windows, macOS) without
# extra dependencies.

set -euo pipefail

echo "=== Breaking-change-without-ADR check ==="

# Find the last release tag (e.g. "v0.1.0-m3"). If none, fall back to the
# root commit so the very first CI run still gets a meaningful check.
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if [ -n "$LAST_TAG" ]; then
  RANGE="${LAST_TAG}..HEAD"
  echo "Checking commits in range: $RANGE (since tag ${LAST_TAG})"
else
  ROOT=$(git rev-list --max-parents=0 HEAD)
  RANGE="${ROOT}..HEAD"
  echo "No release tag found; checking all commits since root ${ROOT}"
fi

# --- 1. Count breaking-change commits in the range. -----------------------
# A commit is "breaking" if its subject matches `^<type>!:...` (e.g.
# "feat!:" or "fix!:") OR if its body contains a "BREAKING CHANGE:" line.
# `--extended-regexp` + multiple `--grep` gives us OR semantics across
# the entire commit message (subject + body), which is what we want.
BREAKING_LIST=$(git log "$RANGE" --extended-regexp \
  --grep='^[a-z]+(\([^)]+\))?!:' \
  --grep='^BREAKING CHANGE:' \
  --format='%h %s' || true)

# Counting with `wc -l` is the most portable approach. `grep -c .` would
# also work but requires a non-empty stdin.
BREAKING_COUNT=0
if [ -n "$BREAKING_LIST" ]; then
  BREAKING_COUNT=$(printf '%s\n' "$BREAKING_LIST" | wc -l | tr -d ' ')
fi

# --- 2. Count new ADR files in the range. --------------------------------
# "New" means *added* in the range (--diff-filter=A). The first ever CI run
# (no tag) treats every existing ADR as new, so the check is sensible
# from day one.
NEW_ADR_LIST=""
if [ -n "$LAST_TAG" ]; then
  NEW_ADR_LIST=$(git diff --name-only --diff-filter=A "$RANGE" -- 'docs/adr/*.md' || true)
else
  # No tag: every committed ADR is "new" relative to the root.
  NEW_ADR_LIST=$(git ls-files 'docs/adr/*.md' || true)
fi
NEW_ADR_COUNT=0
if [ -n "$NEW_ADR_LIST" ]; then
  NEW_ADR_COUNT=$(printf '%s\n' "$NEW_ADR_LIST" | wc -l | tr -d ' ')
fi

# --- 3. Report and gate. --------------------------------------------------
echo "Breaking-change commits: ${BREAKING_COUNT}"
echo "New ADR files:            ${NEW_ADR_COUNT}"

if [ "$BREAKING_COUNT" -gt 0 ] && [ "$NEW_ADR_COUNT" -eq 0 ]; then
  echo ""
  echo "::error title=Breaking change without ADR::Found ${BREAKING_COUNT} breaking change(s) but no new ADR was added under docs/adr/. Per AGENTS.md §11 (Communication Norms) and the Architecture Decision Record process, every breaking change must be accompanied by an ADR in the same PR."
  if [ -n "$BREAKING_LIST" ]; then
    echo ""
    echo "Breaking commits:"
    printf '%s\n' "$BREAKING_LIST" | sed 's/^/  - /'
  fi
  echo ""
  echo "Fix: create a new ADR under docs/adr/ describing the architectural decision, then reference it from the PR description."
  exit 1
fi

echo "OK"
