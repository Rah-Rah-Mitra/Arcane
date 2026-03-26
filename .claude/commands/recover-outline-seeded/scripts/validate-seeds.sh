#!/usr/bin/env bash
# validate-seeds.sh — validate a seed JSON file before using it for recovery
# Usage: bash validate-seeds.sh <seeds.json>
set -euo pipefail

SEEDS="${1:?usage: validate-seeds.sh <seeds.json>}"

if [[ ! -r "$SEEDS" ]]; then
  echo "ERROR: cannot read file: $SEEDS" >&2
  exit 1
fi

python3 - "$SEEDS" <<'EOF'
import sys, json

path = sys.argv[1]
try:
    data = json.load(open(path))
except json.JSONDecodeError as e:
    print(f"ERROR: invalid JSON — {e}", file=sys.stderr)
    sys.exit(1)

if not isinstance(data, list):
    print("ERROR: root must be a JSON array", file=sys.stderr)
    sys.exit(1)

errors = []
for i, entry in enumerate(data):
    if not isinstance(entry, dict):
        errors.append(f"  entry {i}: must be an object")
        continue
    if "title" not in entry or not isinstance(entry["title"], str):
        errors.append(f"  entry {i}: missing/invalid 'title' (must be string)")
    if "page" not in entry or not isinstance(entry["page"], int) or entry["page"] < 1:
        errors.append(f"  entry {i}: missing/invalid 'page' (must be int >= 1)")
    depth = entry.get("depth", 1)
    if not isinstance(depth, int) or depth < 1:
        errors.append(f"  entry {i}: invalid 'depth' (must be int >= 1)")

if errors:
    print("Validation FAILED:")
    for e in errors:
        print(e)
    sys.exit(1)

print(f"✓ Valid seed file: {len(data)} entries")
print(f"  depth range: {min(e.get('depth',1) for e in data)}–{max(e.get('depth',1) for e in data)}")
print(f"  page range:  {min(e['page'] for e in data)}–{max(e['page'] for e in data)}")
for e in data[:5]:
    print(f"  p{e['page']:>4}  d{e.get('depth',1)}  {e['title'][:60]}")
if len(data) > 5:
    print(f"  ... and {len(data)-5} more")
EOF
