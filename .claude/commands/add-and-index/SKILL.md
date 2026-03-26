# Skill: add-and-index

Add a PDF source to an Arcane project and index it for full-text search.

## When to use

The user wants to "add a PDF", "add a book", "import a paper", or "add to project".

## Steps Claude must follow

1. **Gather required info** (ask if not provided):
   - Project name
   - PDF file path
   - Is it a textbook? (needs chunking) or a report/paper?

2. **Probe the PDF**:
   ```bash
   arcane analyze probe "<pdf>" --json
   ```
   Report `document_kind`, `has_outlines`, `total_pages` to the user.

3. **Determine start page** (textbooks only):
   If the user doesn't know the offset:
   ```bash
   arcane analyze offset "<pdf>" --json
   ```
   If offset found: `start_page = offset` (0-based physical page where Arabic page 1 starts).

4. **Create project if needed**:
   ```bash
   arcane new "<project>"
   ```
   (If project already exists, this is a no-op.)

5. **Add the source**:
   ```bash
   # Textbook with known offset:
   arcane add "<project>" "<pdf>" --textbook --start-page $OFFSET

   # Textbook with TOC pages (for future batch recovery):
   arcane add "<project>" "<pdf>" --textbook --start-page $OFFSET \
     --toc-start-page N --toc-end-page M

   # Report or paper:
   arcane add "<project>" "<pdf>" --type report

   # With tags:
   arcane add "<project>" "<pdf>" --tag "machine-learning" --tag "2024"
   ```

6. **Index for search**:
   ```bash
   arcane reindex
   ```

7. **Verify**:
   ```bash
   arcane search "key term from document" --project "<project>"
   ```

8. Fill in `template.md` and present the summary.

## Source type reference

| Flag / `--type` | Use case |
|----------------|---------|
| `--textbook` | Book that should be split into chapters |
| `--type report` | Technical report (no chunking) |
| `--type paper` | Research paper (no chunking) |
| `--type cheatsheet` | Quick reference card |

## Notes

- CAS deduplication is automatic — adding the same file twice is safe.
- `--toc-start-page`/`--toc-end-page` are optional but enable `arcane recover-project` later.
- Reindex is idempotent.
