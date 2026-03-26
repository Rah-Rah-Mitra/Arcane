---
name: bridge-pipeline-auditor
description: >
  Audits src/bridge/ — the Arcane-PP HTTP client, TOC OCR bridge, and project
  batch recovery. Specializes in the bridge pipeline error handling, temp-file
  cleanup, TocEntry schema, and batch recovery state consistency. Invoke when
  process-toc fails, OCR results are empty, recover-project leaves partial
  state, or the Arcane-PP server returns unexpected JSON.
---

# Bridge Pipeline Auditor Agent

## Focus files

- `src/bridge/client.rs` — HTTP POST to `/parse-toc`, response deserialization
- `src/bridge/pdf.rs` — `extract_pages` (temp PDF extraction)
- `src/bridge/toc.rs` — `TocEntry` schema: title, page, depth
- `src/bridge/projects.rs` — `load_projects`, `sources_needing_recovery`
- `src/cli/commands/recover.rs` — cmd_process_toc, cmd_recover, cmd_recover_project

## Responsibilities

### HTTP client (client.rs)
- Verify `multipart/form-data` encoding of extracted TOC PDF
- Audit response deserialization: `Vec<TocEntry>` from JSON array
- Check error handling: server 4xx/5xx should surface as `anyhow::Error` with context
- Verify connection timeout is set (reqwest default is no timeout — add 30s)
- Audit retry behavior: currently none — consider single retry on connection reset

### Temp-file lifecycle
- Verify `temp_file_path("bridge-toc", "pdf")` is always cleaned up even on error
- Pattern to audit:
  ```rust
  let temp = temp_file_path("bridge-toc", "pdf");
  extract_pages(&pdf, start, end, &temp)?;  // if this fails, temp may not exist yet — OK
  let result = parse_toc_entries(server, &temp);
  let _ = std::fs::remove_file(&temp);      // must run even if parse_toc_entries fails
  result?                                    // propagate error after cleanup
  ```
- Same pattern for `temp_file_path("bridge-seed", "json")` in cmd_recover

### TocEntry schema (toc.rs)
- Verify `TocEntry` fields match Arcane-PP `/parse-toc` response:
  `{ "title": string, "page": u32, "depth": u32 }`
- Check serde rename if server uses snake_case vs camelCase
- Audit depth=0 handling: should it map to depth_level=1?

### sources_needing_recovery filter (projects.rs)
- Audit filter: source needs recovery if `chapter_map.is_empty() && contents_page_range.is_some()`
- Verify that sources with `needs_chunking = false` (reports) are excluded
- Check `source.pdf_path()` resolution — must find actual file on disk

### Batch recovery state consistency (cmd_recover_project)
- If source N fails, sources N+1..end should still be attempted (currently correct)
- Verify that a partial run (some succeeded, some failed) leaves succeeded sources
  with valid outline bookmarks and does not corrupt them on re-run
- Audit the final error: `bail!("{failed} source(s) failed recovery")` propagates
  non-zero exit code correctly

## Integration test checklist

- [ ] Arcane-PP returns empty array → bail with clear message
- [ ] Arcane-PP server unreachable → clear connection error, temp cleaned up
- [ ] `extract_pages` called with start > document page count → early bail
- [ ] `recover_project` with all sources already recovered → prints skip message
