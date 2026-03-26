#!/usr/bin/env bash
# detect-offset.sh — run all offset detection strategies and compare results
# Usage: bash detect-offset.sh <pdf-path> [toc-pages]
set -euo pipefail

PDF="${1:?usage: detect-offset.sh <pdf-path> [toc-pages]}"
TOC_PAGES="${2:-}"

echo "=== Page Offset Detection: $PDF ==="
echo ""

# Strategy 1: automatic (no TOC hint)
echo "--- Strategy: automatic ---"
arcane analyze offset "$PDF" --json 2>/dev/null || echo "null"
echo ""

# Strategy 2: with TOC pages (if provided)
if [[ -n "$TOC_PAGES" ]]; then
  echo "--- Strategy: TOC-assisted (pages $TOC_PAGES) ---"
  arcane analyze offset "$PDF" --toc-pages "$TOC_PAGES" --json 2>/dev/null || echo "null"
  echo ""

  # Strategy 3: RANSAC sync-pages
  echo "--- Strategy: RANSAC sync-pages (pages $TOC_PAGES) ---"
  arcane analyze sync-pages "$PDF" --toc-pages "$TOC_PAGES" --json 2>/dev/null || echo "null"
  echo ""
fi

echo "=== Summary ==="
echo "Pick the offset value with the highest confidence."
echo "Pass it to recover-outline as: --page-one <offset+1>"
