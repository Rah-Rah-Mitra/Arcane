#!/usr/bin/env bash
# tune-ratio.sh — try a range of min-font-ratio values to find the best fit
# Usage: bash tune-ratio.sh <pdf-path> [depth]
set -euo pipefail

PDF="${1:?usage: tune-ratio.sh <pdf-path> [depth]}"
DEPTH="${2:-2}"

echo "Tuning min-font-ratio for: $PDF"
echo "Depth: $DEPTH"
echo ""

for RATIO in 1.05 1.10 1.15 1.20 1.25 1.30 1.40; do
  COUNT=$(arcane recover-outline "$PDF" \
    --dry-run --depth "$DEPTH" --min-font-ratio "$RATIO" --json 2>/dev/null \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d.get('headings', [])))" 2>/dev/null || echo "?")
  printf "  ratio=%-5s  headings=%s\n" "$RATIO" "$COUNT"
done

echo ""
echo "Pick the ratio that gives the expected number of chapter headings."
echo "Then run: arcane recover-outline \"$PDF\" --min-font-ratio <chosen> --depth $DEPTH --dry-run"
