# Skill: merge-split-pdf

Merge multiple PDFs into one, or split a PDF into parts by page range.
All operations are lossless (no re-encoding).

## When to use

- User says "merge", "combine", "join" PDFs → use merge
- User says "split", "extract pages", "separate chapters" → use split

## Merge steps

1. **Confirm all input PDFs are readable**:
   ```bash
   for f in "$@"; do arcane analyze probe "$f" --json | grep document_kind; done
   ```

2. **Merge**:
   ```bash
   arcane pdf merge "<output.pdf>" "<input1.pdf>" "<input2.pdf>" ...
   ```

3. **Verify**:
   ```bash
   arcane analyze outline "<output.pdf>"
   arcane analyze probe "<output.pdf>"
   ```

## Split steps

1. **Find chapter boundary pages** (if splitting along chapters):
   ```bash
   arcane analyze outline "<pdf>" --depth 1
   arcane analyze offset "<pdf>" --json   # if printed pages differ from physical
   ```

2. **Split**:
   ```bash
   # Multiple ranges (1-based):
   arcane pdf split "<pdf>" --output-dir "./parts" "1-45" "46-102" "103-199"

   # Single page:
   arcane pdf split "<pdf>" --output-dir "./parts" "42"
   ```

3. **For a single range** (simpler base command):
   ```bash
   arcane pdf extract-pages "<pdf>" --start N --end M "<output.pdf>"
   ```

4. **Verify each part**:
   ```bash
   arcane analyze probe "./parts/part_0001.pdf"
   ```

5. Fill in `template.md` and present the summary.

## Choosing between split and extract-pages

| Use case | Command |
|----------|---------|
| Multiple ranges at once | `arcane pdf split` |
| Single range / TOC extraction | `arcane pdf extract-pages` |
| Along outline boundaries | `arcane analyze outline` first, then `split` |
