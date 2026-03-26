#!/usr/bin/env bash
# check-server.sh — verify Arcane-PP server is reachable
# Usage: bash check-server.sh [server-url]
set -euo pipefail

SERVER="${1:-http://localhost:5000}"

echo "Checking Arcane-PP server at: $SERVER"

# Try a simple HTTP GET (the server may not support GET /health but will at least respond)
STATUS=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$SERVER/" 2>/dev/null || echo "000")

if [[ "$STATUS" == "000" ]]; then
  echo "ERROR: Cannot reach $SERVER (connection refused or timeout)" >&2
  echo ""
  echo "To start Arcane-PP locally:"
  echo "  cd /path/to/arcane-pp && python app.py"
  echo ""
  echo "Or specify a different server with --server http://host:port"
  exit 1
fi

echo "✓ Server responded (HTTP $STATUS)"
echo ""
echo "Server is reachable. You can now run:"
echo "  arcane process-toc <pdf> --toc-pages \"N-M\" --server $SERVER"
echo "  arcane recover <pdf> --toc-pages \"N-M\" --server $SERVER"
