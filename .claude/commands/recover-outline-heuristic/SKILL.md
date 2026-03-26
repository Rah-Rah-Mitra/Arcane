# Skill: recover-outline-heuristic

Recover PDF outline bookmarks using font-size heuristics when no bookmarks exist
and no reference PDF or TOC seed is available.

## When to use

The user has a PDF with no bookmarks and wants to recover them using font analysis.
Use when they say "recover outline", "add bookmarks", "fix chapters" — and no
reference PDF or Arcane-PP server is mentioned. Prefer this over the bridge workflow
for digitally-typeset PDFs with consistent font sizes.

## Steps Claude must follow

1. **Probe the PDF**:
   ```bash
   arcane analyze probe "<pdf>" --json
   ```
   Abort if `document_kind == "Scanned"`. Warn if `has_outlines == true` (already has bookmarks).

2. **Inspect font clusters and body size**:
   ```bash
   arcane analyze layout "<pdf>" --json
   ```
   Note `body_font_size` and the cluster sizes. Choose `--min-font-ratio` so that
   headings (typically 1.2–1.5× body) are captured but running headers (often same
   size as body) are not.

3. **Detect page offset** (if applicable):
   ```bash
   arcane analyze offset "<pdf>" --json
   ```
   If offset is found, note the `offset` value. Will be passed as `--page-one` below.

4. **Dry-run — preview detected headings**:
   ```bash
   arcane recover-outline "<pdf>" --dry-run --depth 2 --min-font-ratio 1.2
   ```
   Show the heading table to the user. If results look wrong, adjust `--min-font-ratio`
   and repeat. Also try `--toc-pages "N-M"` if the user can identify the TOC pages.

5. **Write the recovered PDF** (only after user confirms):
   ```bash
   arcane recover-outline "<pdf>" --output "<fixed.pdf>" --depth 2 --min-font-ratio 1.2
   ```

6. **Verify injected bookmarks**:
   ```bash
   arcane analyze outline "<fixed.pdf>" --depth 2
   ```

7. Fill in `template.md` and present the summary.

## Parameter selection guide

| Observation | Recommendation |
|-------------|----------------|
| Too many headings detected | Increase `--min-font-ratio` (try 1.3) |
| Too few headings | Decrease `--min-font-ratio` (try 1.1) or add `--toc-pages` |
| Wrong page numbers | Pass `--page-one N` with the offset value from step 3 |
| Section headings missing | Use `--depth 2` |
| Confident results needed fast | Add `--toc-pages "N-M"` for fuzzy-match boost (+0.20 confidence) |
