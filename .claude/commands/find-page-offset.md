# Workflow: find-page-offset

Determine the physical-to-logical page offset for a PDF — the integer delta
between printed page numbers (e.g. "page 1") and physical PDF page indices
(0-based).  Required when front matter uses Roman numerals or the PDF has
extra blank pages before content.

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane analyze offset` | `pdf::offset::calculate_offset` (3-strategy cascade) |
| 3 | `arcane analyze layout` | `pdf::layout::analyze_layout` (find TOC page candidates) |
| 4 | `arcane analyze sync-pages` | RANSAC heading↔TOC consensus |

## Offset detection strategies (automatic cascade)

1. **PageLabels** (confidence 0.95) — reads `/PageLabels` number tree
2. **TOC Matching** (confidence 0.6–0.9) — matches TOC entries vs page text
3. **Page-Number Detection** (confidence 0.6–0.8) — finds printed numbers in headers/footers

## Steps

```bash
# 1. Confirm PDF is text-based
arcane analyze probe book.pdf

# 2. Try automatic offset detection
arcane analyze offset book.pdf --json

# If successful: use the reported offset value as --page-one in recover-outline

# 3. If automatic fails — find the TOC pages first
arcane analyze layout book.pdf --json | jq '[.anchors[] | select(.kind == "TocEntry")] | .[0].page_index'

# 4. Retry with explicit TOC pages
arcane analyze offset book.pdf --toc-pages "7-18" --json

# 5. Validate with RANSAC consensus
arcane analyze sync-pages book.pdf --toc-pages "7-18" --json
```

## Reading the output

```json
{
  "offset": 18,        // physical_index = logical_page + offset
  "confidence": 0.95,
  "method": "PageLabels",
  "evidence": [...]
}
```

`offset = 18` means: if the book says "page 1", it's at physical index 19
(1-based).  Pass `--page-one 19` to `arcane recover-outline`.

## Offset strategy confidence guide

| Confidence | Reliability |
|-----------|-------------|
| ≥ 0.90 | Use directly |
| 0.70–0.89 | Verify with `sync-pages` |
| < 0.70 | Override manually with `--page-one` |
