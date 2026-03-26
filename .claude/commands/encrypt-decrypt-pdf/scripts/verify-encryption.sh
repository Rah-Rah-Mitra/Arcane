#!/usr/bin/env bash
# verify-encryption.sh — check whether a PDF is password-protected
# Usage: bash verify-encryption.sh <pdf-path>
# Exits 0 if encrypted, 1 if not encrypted, 2 on error
set -euo pipefail

PDF="${1:?usage: verify-encryption.sh <pdf-path>}"

if [[ ! -r "$PDF" ]]; then
  echo "ERROR: cannot read file: $PDF" >&2
  exit 2
fi

# Try to probe without a password — encrypted PDFs will fail or return minimal info
RESULT=$(arcane analyze probe "$PDF" --json 2>&1 || true)

if echo "$RESULT" | grep -qi "encrypt\|password\|decrypt\|locked"; then
  echo "✓ PDF appears to be password-protected"
  exit 0
elif echo "$RESULT" | grep -q "document_kind"; then
  echo "PDF is NOT password-protected (opens freely)"
  exit 1
else
  echo "Could not determine encryption status. Raw output:"
  echo "$RESULT"
  exit 2
fi
