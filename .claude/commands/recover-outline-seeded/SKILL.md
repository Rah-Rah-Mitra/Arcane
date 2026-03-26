# Skill: recover-outline-seeded

Recover PDF outline bookmarks using a reference PDF or known chapter list (seeds).
Seeds provide ground-truth chapter titles; Arcane locates them in the target via
fuzzy matching, votes for the physical page offset, and injects bookmarks.

## When to use

- The user has another edition of the same book with working bookmarks (`--seed-pdf`)
- The user has a JSON list of chapter titles and logical page numbers (`--seed-file`)
- Heuristic recovery gave poor results and known chapter names are available

## Steps Claude must follow

1. **Probe the target PDF**:
   ```bash
   arcane analyze probe "<target.pdf>" --json
   ```
   Abort if `document_kind == "Scanned"`.

2. **If using a reference PDF** — verify it has bookmarks:
   ```bash
   arcane analyze outline "<reference.pdf>" --depth 2
   ```
   If empty, the reference is unusable; fall back to `recover-outline-bridge` or ask for a JSON seed file.

3. **If using a JSON seed file** — validate the format:
   ```bash
   bash .claude/commands/recover-outline-seeded/scripts/validate-seeds.sh "<seeds.json>"
   ```
   Expected: `[{"title": "...", "page": N, "depth": D}, ...]` (page = 1-based logical).

4. **Dry-run with seeds**:
   ```bash
   # Reference PDF:
   arcane recover-outline "<target.pdf>" --seed-pdf "<reference.pdf>" --dry-run --depth 2

   # OR JSON seed file:
   arcane recover-outline "<target.pdf>" --seed-file "<seeds.json>" --dry-run --depth 2
   ```
   Show the seed verification table (Confirmed/Estimated/OutOfRange counts).
   If many seeds are `OOR` or `EST`, ask the user for `--page-one N` or `--anchor L:P` corrections.

5. **Write the recovered PDF** (only after user confirms):
   ```bash
   arcane recover-outline "<target.pdf>" --seed-pdf "<reference.pdf>" \
     --output "<fixed.pdf>" --depth 2
   ```

6. **Verify**:
   ```bash
   arcane analyze outline "<fixed.pdf>" --depth 2
   ```

7. Fill in `template.md` and present the summary.

## Drift correction flags

- `--page-one N` — override offset (physical page where printed page 1 starts)
- `--seed-tolerance N` — ±N page search window per seed (default 5)
- `--offset-tolerance N` — ±N offset voting range (default 50)
- `--anchor L:P` — per-segment correction; repeat for each discontinuity

## Seed status guide

| Status | Meaning | Action |
|--------|---------|--------|
| `OK` (Confirmed) | Found on expected page | None needed |
| `EST` (Estimated) | Interpolated from neighbours | Increase `--seed-tolerance` |
| `OOR` (OutOfRange) | Page outside document bounds | Provide `--page-one` or `--anchor` |
