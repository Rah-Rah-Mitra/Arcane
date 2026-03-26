# Skill: chunk-textbook

Split a textbook PDF into per-chapter files using its embedded or recovered outline.

## When to use

The user has a textbook PDF in an Arcane project and wants it split into one PDF per chapter. Run this skill when they say "chunk", "split into chapters", or "split by chapter".

## Steps Claude must follow

1. **Ask for the project name and source title** if not provided.

2. **Probe the PDF** to confirm it is text-based:
   ```bash
   arcane analyze probe "<pdf-path>" --json
   ```
   - If `document_kind` is `Scanned` → stop and tell the user outline recovery is required first.

3. **Inspect the existing outline**:
   ```bash
   arcane analyze outline "<pdf-path>" --depth 2
   ```
   - If no outlines → tell the user to run `recover-outline-heuristic` or `recover-outline-bridge` first, then return here.

4. **Dry-run to preview chapter boundaries**:
   ```bash
   arcane chunk "<project>" --depth 1 --dry-run
   ```
   Show the user the boundary table and ask: "Does this look correct? Should I proceed?"

5. **Execute chunking** (only after user confirms):
   ```bash
   arcane chunk "<project>" --depth 1
   ```
   If source already chunked and `--force` was not requested, inform the user and offer to re-chunk with `--force`.

6. **Verify output**:
   ```bash
   arcane list-chunks "<project>"
   ```
   Report the chunk count and list the files.

7. Fill in `template.md` and present the summary to the user.

## Flags to mention

- `--depth 2` — includes section-level headings
- `--source "Title"` — chunk only one specific source
- `--force` — overwrite existing chunks

## Error handling

| Problem | Action |
|---------|--------|
| No outlines found | Direct to recover-outline-heuristic or recover-outline-bridge |
| Source is scanned | Stop; explain OCR is not supported |
| Wrong boundaries | Suggest `--depth 2` or running `analyze offset` first |
