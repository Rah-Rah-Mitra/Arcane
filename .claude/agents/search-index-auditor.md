---
name: search-index-auditor
description: >
  Audits src/search/ and search-related CLI commands (reindex, freq, search).
  Specializes in Tantivy full-text indexing correctness, index/remove
  idempotency, frequency dictionary generation, and text extraction edge cases.
  Invoke when search returns stale/duplicate results, reindex fails, freq output
  is empty, or text extraction is garbled for certain PDFs.
---

# Search Index Auditor Agent

## Focus files

- `src/search/indexer.rs` — schema, `index_source`, `remove_source`, index lifecycle
- `src/search/query.rs` — `search`, project/source filter construction
- `src/pdf/text.rs` — `extract_all`, `extract_range`
- `src/cli/commands/workflow.rs` — cmd_reindex, cmd_freq, cmd_search

## Responsibilities

### Schema alignment (indexer.rs)
- Verify 6 fields match between schema definition and document building:
  `source_id` (STRING), `project` (STRING), `title` (STRING),
  `chapter` (STRING), `page` (U64 STORED), `body` (TEXT)
- Audit that `source_id` format `"project_name:source_title"` is consistent
  across `index_source`, `remove_source`, and search filter construction

### index_source idempotency
- Verify pattern: `remove_source(source_id)` called before `index_source` in `cmd_reindex`
- Check that `remove_source` deletes ALL documents for a source_id, not just the first
- Audit Tantivy writer commit — must call `writer.commit()` after all docs added

### remove_source correctness
- Audit `DeleteTerm` query on `source_id` field — must use exact STRING match
- Verify writer is committed after delete (otherwise delete is not visible to readers)

### search query (query.rs)
- Audit `BooleanQuery` construction for project/source filters:
  - `Must(TermQuery(project = "MyProject"))` narrows to project
  - `Must(TermQuery(title = "MySource"))` narrows to source
- Verify score computation — body TEXT field should dominate ranking
- Check that empty query string returns no results (not all documents)

### Text extraction (text.rs)
- Audit `extract_all` for lopdf encoding edge cases:
  - PDFDocEncoding vs UTF-16BE in `/ToUnicode` CMaps
  - Ligatures (fi, fl, ff) may appear as single glyphs
  - CID fonts without /ToUnicode map → garbled text (log warning, don't panic)
- Verify `extract_range` page indices are 0-based (matching lopdf `get_pages()`)
- Check that empty pages (images only) produce empty strings, not errors

### Frequency dictionary (freq.rs)
- Audit `build_frequency_dict`: must aggregate across all pages for the project
- Verify stop-word filtering is applied before sorting
- Check output format: one `word\tcount` pair per line, sorted descending
- Audit `write_freq_file`: file should be truncated on overwrite, not appended

## Performance targets

- `extract_all` on a 500-page PDF: < 10s on release build
- `reindex` for 10 sources × 300 pages each: < 60s on release build
- `search` with project filter: < 100ms for any query

## Edge cases to test

- Source with 0 pages (empty PDF) — should index gracefully with 0 pages
- Source title containing `:` — source_id delimiter; verify escaping
- Unicode title with non-ASCII characters — Tantivy STRING field must store verbatim
- Very large source (1000+ pages) — memory usage should stay bounded
