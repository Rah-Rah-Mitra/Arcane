# Workflow: add-and-index

Add a PDF source to a project and index it for full-text search.

## Base operations used

| Step | Command | Operation |
|------|---------|-----------|
| 1 | `arcane analyze probe` | `pdf::probe::probe` |
| 2 | `arcane new` | `storage::ProjectStore` upsert |
| 3 | `arcane add` | `storage::cas::ingest` + SQLite + legacy JSON |
| 4 | `arcane reindex` | `pdf::text::extract_all` + `SearchIndex::index_source` |
| 5 | `arcane search` | `SearchIndex::search` |

## Steps

```bash
# 1. Classify the document
arcane analyze probe paper.pdf

# 2. Create project if needed
arcane new "MyProject"

# 3a. Add a textbook (needs chunking, may have TOC pages)
arcane add "MyProject" textbook.pdf \
  --textbook \
  --start-page 19 \
  --toc-start-page 7 \
  --toc-end-page 18

# 3b. Add a report / paper (no chunking needed)
arcane add "MyProject" paper.pdf --type report

# 3c. Add with tags
arcane add "MyProject" paper.pdf --tag "machine-learning" --tag "2024"

# 4. Rebuild search index
arcane reindex

# 5. Verify search works
arcane search "gradient descent" --project "MyProject"
```

## Source type values

| `--type` value | Meaning |
|----------------|---------|
| `textbook` | Textbook (implies `--textbook`) |
| `report` | Technical report |
| `paper` | Research paper |
| `cheatsheet` | Reference sheet |
| `custom string` | Any label |

## Notes

- CAS deduplication is automatic: adding the same file twice skips the copy.
- `arcane reindex` is idempotent — it removes stale entries before re-indexing.
- Search is scoped to a project with `--project` or to a source with `--source`.
