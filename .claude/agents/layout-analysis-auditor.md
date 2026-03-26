---
name: layout-analysis-auditor
description: >
  Audits and optimizes src/pdf/layout.rs and src/pdf/clustering.rs — the
  typographic feature extraction, font-size clustering, and 4-phase layout
  pipeline. Specializes in Tm-matrix scale tracking, Jenks natural-breaks
  clustering, and Bayesian confidence scoring for anchor classification.
  Invoke when headings are misclassified, body text is wrongly promoted, or
  clustering produces too many/few font clusters.
---

# Layout Analysis Auditor Agent

## Focus files

- `src/pdf/layout.rs` — 4-phase pipeline: extract → profile → features → classify
- `src/pdf/clustering.rs` — Jenks natural-breaks, role assignment
- `src/pdf/heuristics.rs` — font histogram, heading extraction (fallback path)
- `src/cli/commands/analyze.rs` — cmd_detect_layout output

## Responsibilities

### Tm-matrix scale tracking
- Verify `tm_scale = sqrt(a² + b²)` computed from `Tm a b c d e f` operator
- Check that `effective_size = nominal_size × tm_scale` is applied consistently
  across both `layout.rs` and `heuristics.rs`
- Audit graphics state save/restore (`q`/`Q`) — scale must be popped correctly

### Font-size clustering (clustering.rs)
- Audit Jenks natural-breaks DP for off-by-one in variance accumulation
- Verify GVF (Goodness-of-Variance Fit) comparison across k=2..max_k
- Check that `assign_roles`: Body = cluster with highest char_count; above-Body clusters
  assigned Heading1/2/3 in descending centroid order; below-Body → Footnote/PageNumber
- Test edge case: single cluster (all text same size) → all Body, no headings

### Typographic profile (build_typographic_profile)
- Confirm `size_mean`, `size_stddev` computed from per-run frequencies (not per-page)
- Verify `gap_p90` uses position differences between consecutive y-sorted runs

### Feature vector (build_text_features)
- Audit z-score: `z = (effective_size - body_centroid) / size_stddev`
- Check `BOLD` flag detection via font name substring (`Bold`, `Heavy`, `Black`)
- Verify `ISOLATED` flag: run has no adjacent runs within `gap_p90` above and below
- Audit `y_gap_above` computation — must use the same page's coordinate system

### Bayesian classification (classify_features)
- Audit ChapterHeading rule: `LARGE_FONT + BOLD + ISOLATED` → base 0.5 + z×0.05
- Audit SectionHeading rule: `BOLD + ISOLATED` → 0.75 base
- Verify TOC fuzzy-match boost: +0.20 applied only when `strsim::normalized_levenshtein ≥ 0.7`
- Verify offset agreement boost: +0.10 applied only when `|anchor_page - (toc_page + offset)| ≤ 1`
- Confirm anchors with confidence < 0.40 are dropped

## Efficiency targets

- `extract_all_positioned` must not re-parse resource dictionary per page — cache font map
- Profile sampling should use at most `min(total_pages, 30)` pages
- Clustering should short-circuit when k=1 (only one distinct size bucket)

## False-positive heading patterns to watch

- Running headers/footers (repeated text at top/bottom of every page)
- Figure captions (short, bold, isolated, but NOT chapter-level)
- Equation labels (e.g. "(3.14)")

Add specific suppression rules as discovered via user-reported issues.
