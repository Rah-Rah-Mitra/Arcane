# Skill: recover-outline-bridge

Recover PDF outline bookmarks by OCR-ing the printed table of contents via
the Arcane-PP server, then using the parsed entries as seeds for recovery.

## When to use

- The PDF has no bookmarks and no reference edition is available
- The printed TOC is the only reliable source of chapter structure
- The user mentions "Arcane-PP", "OCR", "TOC pages", or "bridge"
- Heuristic recovery (`recover-outline-heuristic`) gave poor results

## Prerequisites

- Arcane-PP server must be running. Verify:
  ```bash
  bash .claude/commands/recover-outline-bridge/scripts/check-server.sh
  ```
- User must provide (or agree to identify) the TOC page range.

## Steps Claude must follow

1. **Probe the PDF**:
   ```bash
   arcane analyze probe "<pdf>" --json
   ```
   Abort if `document_kind == "Scanned"`.

2. **Identify TOC page range** — ask the user if unknown. Alternatively, detect automatically:
   ```bash
   arcane analyze layout "<pdf>" --json | python3 -c \
     "import sys,json; d=json.load(sys.stdin); \
      toc=[a for a in d['anchors'] if a['kind']=='TocEntry']; \
      print(f\"TOC pages: {min(a['page_index']+1 for a in toc)}-{max(a['page_index']+1 for a in toc)}\") if toc else print('No TOC anchors detected')"
   ```

3. **Manual path — extract and inspect** (recommended for first time):
   ```bash
   arcane pdf extract-pages "<pdf>" --start N --end M toc-extract.pdf
   arcane process-toc "<pdf>" --toc-pages "N-M" --output seeds.json
   ```
   Show the user the `seeds.json` content. Ask: "Does this look like the correct chapter list?"
   Then run recovery with the seeds:
   ```bash
   arcane recover-outline "<pdf>" --seed-file seeds.json --toc-pages "N-M" \
     --dry-run --depth 2
   ```

4. **Automated path** (when user is confident):
   ```bash
   arcane recover "<pdf>" --toc-pages "N-M" --output "<fixed.pdf>" --depth 2
   ```

5. **For a whole project** (when all sources have TOC ranges set):
   ```bash
   arcane recover-project --project "<name>" --dry-run
   arcane recover-project --project "<name>"
   ```

6. **Verify**:
   ```bash
   arcane analyze outline "<fixed.pdf>" --depth 2
   ```

7. Fill in `template.md` and present the summary.

## Setting TOC ranges for batch recovery

When adding sources, include TOC page ranges so `recover-project` works:
```bash
arcane add "<project>" "<pdf>" --textbook --toc-start-page N --toc-end-page M
```
