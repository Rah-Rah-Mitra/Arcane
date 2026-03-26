#!/usr/bin/env bash
# verify-index.sh — check that a source was successfully indexed
# Usage: bash verify-index.sh <project> <search-term>
set -euo pipefail

PROJECT="${1:?usage: verify-index.sh <project> <search-term>}"
TERM="${2:?usage: verify-index.sh <project> <search-term>}"

echo "Verifying index for project: $PROJECT"
echo "Search term: $TERM"
echo ""

RESULTS=$(arcane search "$TERM" --project "$PROJECT" 2>&1)
COUNT=$(echo "$RESULTS" | grep -c "^\s*[0-9]\+\." || true)

if [[ $COUNT -eq 0 ]]; then
  echo "WARN: No results found for '$TERM' in project '$PROJECT'."
  echo "      The source may not be indexed yet. Run: arcane reindex"
else
  echo "✓ Found $COUNT result(s) — index is working."
  echo ""
  echo "$RESULTS" | head -10
fi
