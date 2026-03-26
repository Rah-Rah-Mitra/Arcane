#!/usr/bin/env bash
# preflight.sh — verify prerequisites before chunking
# Usage: bash preflight.sh <project> <pdf-path>
set -euo pipefail

PROJECT="${1:?usage: preflight.sh <project> <pdf-path>}"
PDF="${2:?usage: preflight.sh <project> <pdf-path>}"

echo "=== Preflight check ==="

# 1. arcane binary
if ! command -v arcane &>/dev/null; then
  echo "ERROR: arcane not found in PATH. Run: cargo install --path ." >&2
  exit 1
fi
echo "✓ arcane binary found"

# 2. PDF exists and is readable
if [[ ! -r "$PDF" ]]; then
  echo "ERROR: cannot read PDF at: $PDF" >&2
  exit 1
fi
echo "✓ PDF readable"

# 3. Project exists
if ! arcane list 2>/dev/null | grep -q "$PROJECT"; then
  echo "WARN: project '$PROJECT' not found — it will be created by 'arcane add'"
fi

# 4. PDF classification
KIND=$(arcane analyze probe "$PDF" --json 2>/dev/null | grep -o '"document_kind":"[^"]*"' | cut -d'"' -f4 || echo "unknown")
echo "  document_kind: $KIND"
if [[ "$KIND" == "Scanned" ]]; then
  echo "ERROR: PDF is scanned (image-only). Run recover-outline-bridge first." >&2
  exit 1
fi
echo "✓ PDF is text-based"

# 5. Has outlines?
HAS_OUTLINES=$(arcane analyze probe "$PDF" --json 2>/dev/null | grep -o '"has_outlines":[a-z]*' | cut -d: -f2 || echo "false")
if [[ "$HAS_OUTLINES" != "true" ]]; then
  echo "WARN: PDF has no bookmarks. Run recover-outline-heuristic or recover-outline-bridge first."
fi

echo "=== Preflight passed ==="
