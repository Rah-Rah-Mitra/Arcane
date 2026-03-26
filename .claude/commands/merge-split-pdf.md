# Workflow: merge-split-pdf

Merge multiple PDFs into one, or split a PDF into parts by page range.
Both operations are lossless (no re-encoding, no quality loss).

## Base operations used

| Command | Operation |
|---------|-----------|
| `arcane pdf merge` | `pdf::ops::merge` |
| `arcane pdf split` | `pdf::ops::split` |
| `arcane analyze outline` | Determine chapter boundaries for split points |
| `arcane analyze offset` | Get page offset before splitting |

## Merge

```bash
# Merge two or more PDFs (in order)
arcane pdf merge combined.pdf part1.pdf part2.pdf part3.pdf

# Verify structure of merged result
arcane analyze outline combined.pdf
```

## Split by known ranges

```bash
# Split into three parts (1-based, inclusive)
arcane pdf split book.pdf --output-dir ./parts "1-45" "46-102" "103-199"

# Single page extraction
arcane pdf split book.pdf --output-dir ./parts "7"
```

## Split along chapter boundaries

```bash
# 1. Find chapter boundary pages
arcane analyze outline book.pdf --depth 1

# 2. Note the page ranges from the output, then split
arcane pdf split book.pdf --output-dir ./chapters "1-44" "45-101" "102-198"
```

## Split with page offset

```bash
# If printed page 1 = physical page 19, adjust ranges accordingly
arcane analyze offset book.pdf
# offset = 18 → add 18 to each logical page to get physical page
arcane pdf split book.pdf --output-dir ./chapters "19-62" "63-119"
```

## Extract a single page range (base command)

```bash
# Lower-level: extract pages 7-18 into a new file
arcane pdf extract-pages book.pdf --start 7 --end 18 toc-pages.pdf
```

## Notes

- Page ranges are 1-based (matching what your PDF reader shows)
- `split` output files are named `part_0001.pdf`, `part_0002.pdf`, etc.
- Use `extract-pages` for a single range; use `split` for multiple ranges at once
