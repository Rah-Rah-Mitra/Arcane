# Workflow: recover-outline-seeded

Recover outline bookmarks using a reference PDF or known chapter list as seeds.
Seeds provide ground-truth chapter titles; Arcane locates them in the target PDF
via fuzzy matching, votes for the physical page offset, and injects bookmarks.

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane analyze outline` | verify reference PDF has bookmarks |
| 3 | `arcane recover-outline --seed-pdf` | `pdf::seed::load_seeds_from_pdf` + `pdf::pipeline::recover_outline_seeded` |
|   | OR `--seed-file` | `pdf::seed::load_seeds_from_json` + same pipeline |
| 4 | `arcane analyze outline` | verify injected bookmarks |

## Steps — using a reference PDF

```bash
# 1. Confirm target is text-based
arcane analyze probe target.pdf

# 2. Confirm reference has working bookmarks
arcane analyze outline reference.pdf

# 3. Dry-run with seed reference
arcane recover-outline target.pdf \
  --seed-pdf reference.pdf \
  --dry-run --depth 2

# 4. Write fixed PDF
arcane recover-outline target.pdf \
  --seed-pdf reference.pdf \
  --output fixed.pdf --depth 2

# 5. Verify
arcane analyze outline fixed.pdf
```

## Steps — using a JSON seed file

```bash
# Seed file format: [{"title": "Chapter 1", "page": 19, "depth": 1}, ...]
# Pages are 1-based logical (printed) page numbers.

arcane recover-outline target.pdf \
  --seed-file seeds.json \
  --dry-run --depth 2

arcane recover-outline target.pdf \
  --seed-file seeds.json \
  --output fixed.pdf --depth 2
```

## Key flags

- `--page-one N` — override offset (physical page where printed page 1 starts)
- `--seed-tolerance N` — ±N page search window per seed (default 5)
- `--offset-tolerance N` — ±N offset voting range (default 50, covers up to 50 front-matter pages)
- `--anchor LOGICAL:PHYSICAL` — per-segment drift correction; repeat for multiple discontinuities
- `--toc-pages "N-M"` — supply TOC pages for additional offset confidence

## Seed status legend

| Status | Meaning |
|--------|---------|
| `OK` (Confirmed) | Title found on expected page via fuzzy match |
| `EST` (Estimated) | Interpolated from confirmed neighbours |
| `OOR` (OutOfRange) | Page falls outside document bounds |

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Many `EST` seeds | Increase `--seed-tolerance` |
| Wrong offset detected | Use `--page-one N` to bypass voting |
| Drift mid-book | Add `--anchor LOGICAL:PHYSICAL` at each discontinuity |
