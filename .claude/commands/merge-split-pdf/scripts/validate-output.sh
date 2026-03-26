#!/usr/bin/env bash
# validate-output.sh — verify all PDFs in a directory are valid and report page counts
# Usage: bash validate-output.sh <directory>
set -euo pipefail

DIR="${1:?usage: validate-output.sh <directory>}"

if [[ ! -d "$DIR" ]]; then
  echo "ERROR: not a directory: $DIR" >&2
  exit 1
fi

TOTAL_PAGES=0
ERRORS=0

echo "Validating PDFs in: $DIR"
echo ""

for PDF in "$DIR"/*.pdf; do
  [[ -e "$PDF" ]] || { echo "  (no PDF files found)"; exit 0; }

  RESULT=$(arcane analyze probe "$PDF" --json 2>/dev/null || echo '{"total_pages":0,"document_kind":"Error"}')
  PAGES=$(echo "$RESULT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('total_pages', 0))" 2>/dev/null || echo "?")
  KIND=$(echo "$RESULT"  | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('document_kind', 'Error'))" 2>/dev/null || echo "Error")

  if [[ "$KIND" == "Error" ]]; then
    printf "  %-40s  ERROR\n" "$(basename "$PDF")"
    ERRORS=$((ERRORS + 1))
  else
    printf "  %-40s  %s pages  (%s)\n" "$(basename "$PDF")" "$PAGES" "$KIND"
    TOTAL_PAGES=$((TOTAL_PAGES + PAGES))
  fi
done

echo ""
echo "Total pages: $TOTAL_PAGES"
if [[ $ERRORS -gt 0 ]]; then
  echo "WARN: $ERRORS file(s) had errors"
  exit 1
fi
echo "✓ All files valid"
