# Workflow: recover-outline-bridge

Recover outlines via OCR of TOC pages using the Arcane-PP server.
Best for books where the only reliable chapter list is the printed table of contents.

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane pdf extract-pages` | `bridge::pdf::extract_pages` |
| 3 | `arcane process-toc` | `bridge::client::parse_toc_entries` → seed JSON |
| 4 | `arcane recover-outline --seed-file` | `pdf::pipeline::recover_outline_seeded` |
|   | OR `arcane recover` | steps 2–4 in one command |
| 5 | `arcane analyze outline` | verify injected bookmarks |

## Steps — manual (recommended for inspection)

```bash
# 1. Confirm PDF is text-based
arcane analyze probe book.pdf

# 2. Extract just the TOC pages into a separate PDF
arcane pdf extract-pages book.pdf --start 7 --end 18 toc.pdf

# 3. OCR the TOC pages → seed JSON (requires Arcane-PP running)
arcane process-toc book.pdf --toc-pages "7-18" --output seeds.json

# 4. Inspect seeds.json, then recover with seeds
arcane recover-outline book.pdf \
  --seed-file seeds.json \
  --toc-pages "7-18" \
  --dry-run --depth 2

arcane recover-outline book.pdf \
  --seed-file seeds.json \
  --output fixed.pdf --depth 2

# 5. Verify
arcane analyze outline fixed.pdf
```

## Steps — one-shot (for automation)

```bash
# Combines extract-pages + process-toc + recover-outline in one call
arcane recover book.pdf --toc-pages "7-18" --output fixed.pdf --depth 2

# Or for an entire project (all sources with TOC page ranges set):
arcane recover-project --project "MyProject"
```

## Batch recovery setup

```bash
# Set TOC page ranges when adding sources:
arcane add "MyProject" book.pdf --textbook --toc-start-page 7 --toc-end-page 18

# Then batch-recover all sources in the project:
arcane recover-project --project "MyProject" --dry-run
arcane recover-project --project "MyProject"
```

## Requirements

- Arcane-PP server running at `http://localhost:5000` (default)
- Use `--server http://host:port` to override
