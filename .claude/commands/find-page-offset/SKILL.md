# Skill: find-page-offset

Determine the physical-to-logical page offset for a PDF — the integer delta
between printed page numbers and physical PDF page indices.

## When to use

The user asks "what page does the book start on", "why are chapter pages wrong",
"how do I set --page-one", or "what is the offset". Required before any recovery
or chunking on books with Roman-numeral front matter.

## Steps Claude must follow

1. **Probe the PDF**:
   ```bash
   arcane analyze probe "<pdf>" --json
   ```

2. **Try automatic detection**:
   ```bash
   arcane analyze offset "<pdf>" --json
   ```
   - If `confidence >= 0.90` → use the `offset` value directly. Go to step 5.
   - If `confidence` is 0.70–0.89 → validate with step 3.
   - If `null` or `confidence < 0.70` → proceed to step 3.

3. **Find TOC pages** (if automatic detection failed):
   ```bash
   arcane analyze layout "<pdf>" --json
   ```
   Look in `.anchors[]` for entries with `kind == "TocEntry"`. Note the min/max `page_index + 1`.

4. **Retry with explicit TOC pages**:
   ```bash
   arcane analyze offset "<pdf>" --toc-pages "N-M" --json
   arcane analyze sync-pages "<pdf>" --toc-pages "N-M" --json
   ```
   Compare consensus offsets from both commands.

5. **Report the offset** to the user in plain language:
   "The book's printed page 1 starts at physical page **X** (PDF page index X-1).
    Use `--page-one X` in recover-outline, or `--start-page X-1` in arcane add."

6. Fill in `template.md` and present the summary.

## Interpreting the output

```
offset = physical_start_of_arabic_1 - 1
```
- `offset: 18` → printed "page 1" is at physical PDF page 19 → pass `--page-one 19`
- `offset: 0` → no front matter; printed pages match physical pages

## When all strategies fail

Ask the user: "What physical page number (in your PDF reader) shows the book's printed page 1?"
Their answer becomes `--page-one N` directly.
