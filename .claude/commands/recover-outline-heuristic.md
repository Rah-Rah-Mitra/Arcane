# Workflow: recover-outline-heuristic

Recover outline bookmarks for a PDF that has no `/Outlines` using font-size
heuristics (clustering → typographic profiling → Bayesian classification).

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane analyze layout` | `pdf::layout::analyze_layout` |
| 3 | `arcane analyze offset` | `pdf::offset::calculate_offset` |
| 4 | `arcane recover-outline --dry-run` | `pdf::pipeline::recover_outline` (no write) |
| 5 | `arcane recover-outline` | `pdf::pipeline::recover_outline` + `pdf::heuristics::inject_outlines` |
| 6 | `arcane analyze outline` | verify injected bookmarks |

## Steps

```bash
# 1. Confirm PDF is text-based (not scanned)
arcane analyze probe book.pdf

# 2. Inspect font clusters and anchor candidates to choose min-font-ratio
arcane analyze layout book.pdf --json | jq '.font_clusters, .anchors[:10]'

# 3. Check if page offset can be detected automatically
arcane analyze offset book.pdf --json

# 4. Dry-run — preview detected headings (tune min-font-ratio until results look right)
arcane recover-outline book.pdf --dry-run --depth 2 --min-font-ratio 1.2

# If too few headings:
arcane recover-outline book.pdf --dry-run --min-font-ratio 1.1

# If you know the TOC page range (boosts confidence via fuzzy-match):
arcane recover-outline book.pdf --dry-run --toc-pages "7-18" --min-font-ratio 1.2

# 5. Write the fixed PDF
arcane recover-outline book.pdf --output fixed.pdf --depth 2 --min-font-ratio 1.2

# 6. Verify injected bookmarks
arcane analyze outline fixed.pdf --depth 2
```

## Key flags

- `--min-font-ratio 1.2` — text must be 20% larger than body to count as a heading
- `--depth 2` — inject chapters + sections
- `--toc-pages "N-M"` — supply TOC page range for fuzzy-match boost (+0.20 confidence)
- `--page-one N` — override offset when automatic detection fails
- `--no-inject` — run full pipeline for JSON output without writing
- `--json` — machine-readable full pipeline result

## Tuning guide

| Symptom | Adjustment |
|---------|-----------|
| Too many headings (noise) | Increase `--min-font-ratio` (e.g. 1.3) |
| Too few headings | Decrease `--min-font-ratio` (e.g. 1.1) or add `--toc-pages` |
| Wrong page numbers | Use `--page-one N` or run `arcane analyze offset` first |
| Section headings missing | Use `--depth 2` |
