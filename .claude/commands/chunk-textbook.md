# Workflow: chunk-textbook

Split a textbook PDF into per-chapter files using its embedded outline bookmarks.

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane analyze outline` | `pdf::outlines::extract_chapters_with_depth` |
| 3 | `arcane add` | `storage::cas::ingest` + DB upsert |
| 4 | `arcane chunk --dry-run` | `pdf::engine::detect_boundaries` |
| 5 | `arcane chunk` | `pdf::engine::chunk_pdf` + `pdf::heuristics::inject_outlines` |
| 6 | `arcane list-chunks` | filesystem read |

## Steps

```bash
# 1. Verify the PDF is text-based and has bookmarks
arcane analyze probe book.pdf --json

# 2. Preview outline structure and depth
arcane analyze outline book.pdf --depth 2

# 3. Add source to project (--start-page N if printed page 1 ≠ physical page 1)
arcane add "MyProject" book.pdf --textbook --start-page 19

# 4. Preview chapter boundaries before writing
arcane chunk "MyProject" --depth 1 --dry-run

# 5. Execute chunking
arcane chunk "MyProject" --depth 1

# 6. Verify output
arcane list-chunks "MyProject"
```

## Flags

- `--depth 1` — top-level chapters only
- `--depth 2` — chapters + sections
- `--source "Title"` — chunk a single source; allows per-source depth
- `--force` — re-chunk even if chunks already exist

## If the PDF has no bookmarks

Run `recover-outline-heuristic` or `recover-outline-bridge` first to inject
bookmarks, then re-run this workflow.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| No chapters detected | Run `arcane analyze outline book.pdf` — if empty, recover first |
| Wrong chapter boundaries | Use `--depth 2` or recover with `--toc-pages` |
| Off-by-one pages | Set `--start-page` to the physical page index where page 1 is printed |
