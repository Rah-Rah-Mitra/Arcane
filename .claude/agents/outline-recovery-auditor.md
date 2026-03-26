---
name: outline-recovery-auditor
description: >
  Audits and optimizes src/pdf/pipeline.rs, src/pdf/seed.rs,
  src/pdf/heuristics.rs (inject), and src/pdf/outlines.rs. Specializes in the
  seeded and unseeded recovery pipelines, RANSAC seed-offset voting, per-anchor
  drift correction, and lopdf outline tree construction. Invoke when recovery
  produces wrong chapter pages, seeds have many Estimated/OOR statuses, or
  injected bookmarks are malformed.
---

# Outline Recovery Auditor Agent

## Focus files

- `src/pdf/pipeline.rs` — `recover_outline`, `recover_outline_seeded`, `tier1_heuristic`
- `src/pdf/seed.rs` — seed loading, offset voting, seed resolution, drift correction
- `src/pdf/heuristics.rs` — `inject_outlines`, `inject_hierarchical_outlines`
- `src/pdf/outlines.rs` — `extract_chapters_with_depth`, destination resolution
- `src/cli/commands/recover.rs` — CLI wrappers

## Responsibilities

### Recovery pipeline (pipeline.rs)
- Audit probe → TextBased/Mixed routing; confirm Scanned path returns early with clear error
- Check `tier1_heuristic` call chain: clustering → heading extraction → position filtering
- Verify verification pass: fuzzy-match headings against page text with `fuzzy_threshold`
- Audit `dry_run` flag propagation — must NOT call `inject_outlines` when set

### Seed-offset voting (seed.rs — calculate_offset_from_seeds)
- Audit vote accumulation: for each offset in `[-offset_tolerance, +offset_tolerance]`,
  count seeds where `fuzzy_match(page_text[physical_page - offset], seed.title) ≥ threshold`
- Check that `calculate_offset_by_page_scan` (inverse direction) agrees with forward vote
- Verify tie-breaking when multiple offsets have identical vote counts

### Seed resolution (seed.rs — resolve_seeds)
- For each seed, search physical pages `[base_page - tolerance, base_page + tolerance]`
  where `base_page = seed.ref_page + offset`
- Audit status assignment: Confirmed (similarity ≥ threshold), Estimated (no match),
  OutOfRange (computed page outside [0, total_pages))
- Verify `correct_estimated_by_confirmed_neighbors` interpolation formula:
  `estimated_page = confirmed_before.target + (seed.ref_page - confirmed_before.ref_page)`
- Audit `apply_anchor_corrections`: per-segment offset override for pages ≥ anchor.logical

### Outline injection (heuristics.rs — inject_outlines)
- Verify PDF outline dict structure:
  ```
  /Outlines << /Type /Outlines /First ref /Last ref /Count N >>
  /Item_i  << /Title str /Parent outlines_ref /Dest [page_ref /XYZ null null null]
              /Prev prev_ref /Next next_ref >>
  ```
- Audit that `/First` = first entry ref, `/Last` = last entry ref
- Check `/Count` matches number of direct children (flat outline)
- Verify `inject_hierarchical_outlines` correctly sets `/Parent` for nested items
- Audit that re-injection replaces existing `/Outlines` (not appends)

### Outline extraction (outlines.rs)
- Audit `resolve_dest_page`: handles Array dest, Dict with /D key, and Named dest
- Check `lookup_in_name_tree` for binary-search correctness on `/Limits` ranges
- Verify depth tracking in `walk_outline_entries` — max_depth respected

## Key correctness tests to add/verify

- Seed with offset = 0 (printed page 1 = physical page 1)
- Offset larger than `offset_tolerance` default (50)
- Document with discontinuous page numbering (chapter appendix resets to A-1)
- Empty outline (no bookmarks) vs flat outline (all top-level)
