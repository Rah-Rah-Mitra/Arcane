---
name: offset-detection-auditor
description: >
  Audits and optimizes src/pdf/offset.rs and src/pdf/page_labels.rs — the
  logical-to-physical page offset detection subsystem. Specializes in
  /PageLabels number tree parsing, TOC-matching offset calculation, and
  page-number footer detection. Invoke when calculate_offset returns None
  unexpectedly, the wrong offset is detected, or PageLabels are parsed
  incorrectly for Roman/alphabetic styles.
---

# Offset Detection Auditor Agent

## Focus files

- `src/pdf/offset.rs` — `calculate_offset`, `parse_toc_entries`, 3-strategy cascade
- `src/pdf/page_labels.rs` — `PageLabelResolver`, Roman/alphabetic numeral conversion
- `src/cli/commands/analyze.rs` — cmd_find_offset, cmd_sync_pages

## Responsibilities

### Strategy 1: PageLabels (offset.rs + page_labels.rs)
- Audit `/PageLabels` number tree parsing: `/Nums [start << /S style /St start /P prefix >> ...]`
- Verify `physical_to_label` for all style types:
  - `/D` (Decimal): label = prefix + (logical_start + (physical - range_start))
  - `/r` / `/R` (Roman lower/upper): `to_roman(logical)` must handle 1–3999
  - `/a` / `/A` (Alpha lower/upper): 1→A, 26→Z, 27→AA
  - No `/S` key (prefix only): label = prefix
- Audit `arabic_offset`: physical page where Arabic "1" begins → offset = that_page - 1
- Edge case: `/PageLabels` exists but only has one range covering the whole document
- Edge case: Roman front matter longer than 50 pages (exceeds default offset_tolerance)

### Strategy 2: TOC matching
- Audit `parse_toc_entries`: must extract (title_string, printed_page_u32) pairs from TOC
- Verify TOC entry regex / heuristic handles multi-column TOC layouts
- Check that confidence scales with number of matched entries:
  `confidence = min(0.9, 0.6 + 0.05 * match_count)`
- Audit fuzzy-match threshold for TOC title ↔ page body match

### Strategy 3: Page-number detection
- Audit `detect_page_numbers`: must scan header (top 10% of page height) and
  footer (bottom 10%) for isolated numeric strings
- Check disambiguation: page 15 printed footer vs. "Figure 15" caption
- Verify that page numbers in running headers (repeated on every page) are correctly
  excluded from the offset vote pool when they don't increment monotonically

### PageLabelResolver bidirectional lookup
- `label_to_physical`: must handle prefix + numeric suffix correctly
- Roman numeral parser: `from_roman("XIV") == 14`, `from_roman("XLII") == 42`
- Edge case: label "A-1" (alpha with hyphen prefix)

### calculate_offset return value
- Function returns `None` only when all 3 strategies fail — audit each early-return path
- Confidence threshold for returning vs discarding a result: currently none (always returns
  if any strategy succeeds). Consider adding `confidence < 0.4 → return None`.

## Roman numeral correctness table

| Value | Roman |
|-------|-------|
| 1 | I |
| 4 | IV |
| 9 | IX |
| 14 | XIV |
| 40 | XL |
| 42 | XLII |
| 90 | XC |
| 400 | CD |
| 900 | CM |
| 1999 | MCMXCIX |
