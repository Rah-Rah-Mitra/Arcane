# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Always use --release for PDF work (10-100x faster than debug)
cargo build --release
cargo install --path .         # Install as system command

# Testing
cargo test                     # Run all tests
cargo test -- --nocapture      # With output
cargo test test_name           # Specific test

# Code quality (run before committing)
cargo fmt
cargo clippy

# Debug output for PDF operations
RUST_LOG=arcane=debug arcane chunk "ProjectName"
```

## Three-Tier Command Architecture

Commands are organized into three tiers. High-level operations are built by composing lower tiers.

```
Tier 0 — Base PDF operations     arcane pdf <op>
  merge, split, rotate, protect, unlock, inject-outlines, extract-pages

Tier 1 — Analysis / inspection   arcane analyze <op>
  probe, outline, layout, offset, sync-pages

Tier 2 — Project workflows       arcane <workflow>
  chunk, recover-outline, recover, recover-project, process-toc,
  search, reindex, freq, tui, watch

Project management               arcane <cmd>
  new, list, show, add, remove, tag, untag, list-chunks
```

**Backward compat:** the old flat commands (`arcane merge`, `arcane probe`, etc.) still work but are hidden from `--help`. Prefer the namespaced forms.

### Workflow compositions (see `.claude/commands/` for step-by-step guides)

```
chunk           = analyze outline → engine::detect_boundaries → engine::chunk_pdf
                  → pdf::inject-outlines (if chapter_map set)

recover-outline = analyze probe → analyze layout → analyze offset
                  → pipeline::recover_outline → pdf::inject-outlines

recover         = pdf::extract-pages → process-toc → recover-outline (seeded)

recover-project = [for each source needing recovery] recover

reindex         = text::extract_all + SearchIndex::index_source (per source)
```

## CLI Module Layout

```
src/cli/
  mod.rs              — Commands, PdfCommands, AnalyzeCommands enums (Clap)
  output.rs           — Table formatting helpers
  commands/
    mod.rs            — Re-exports all pub symbols from sub-modules
    helpers.rs        — parse_anchor_pair (pub), parse_page_range, parse_page_ranges,
                        parse_toc_range_1based, temp_file_path, resolve_arcane_data
    project.rs        — cmd_list, cmd_new, cmd_show, cmd_add, cmd_remove,
                        cmd_tag, cmd_untag, cmd_list_chunks
    pdf_ops.rs        — cmd_merge, cmd_split, cmd_rotate, cmd_protect, cmd_unlock,
                        cmd_inject_outlines, cmd_extract_pages
    analyze.rs        — cmd_probe, cmd_outline, cmd_detect_layout, cmd_find_offset,
                        cmd_sync_pages, SyncMatch, PageSyncResult
    recover.rs        — cmd_recover_outline, cmd_process_toc, cmd_recover,
                        cmd_recover_project
    workflow.rs       — cmd_chunk, cmd_search, cmd_freq, cmd_reindex, cmd_tui, cmd_watch
```

**Key constraint:** `parse_anchor_pair` must stay `pub` in `helpers.rs` because `src/cli/mod.rs` references it as `crate::cli::commands::parse_anchor_pair` in a `#[arg(value_parser = ...)]` attribute.

## PDF Engine Architecture

**Data flow:** `src/main.rs` → `cli/commands/*` → `pdf/*` modules → `lopdf::Document`

**Storage layout** (`~/Arcane/`):
```
arcane.db               # SQLite: projects, sources, tags, chunks
CAS/{prefix}/{hash}/    # Content-addressed blob store (BLAKE3)
search_index/           # Tantivy full-text search index
Library/<Project>/
  Originals/            # Symlinks (Unix) / copies (Windows)
  Chunks/<Source>/      # Per-chapter PDFs
```

### PDF modules (`src/pdf/`)

| Module | Lines | Role |
|--------|-------|------|
| `engine.rs` | ~515 | `detect_boundaries`, `chunk_pdf`, `boundaries_to_ranges` |
| `ops.rs` | ~330 | merge, split, rotate, encrypt, decrypt (all lossless) |
| `outlines.rs` | ~380 | Extract `/Outlines` tree with depth; resolve named destinations |
| `heuristics.rs` | ~756 | Font histogram, heading extraction (fallback), `inject_outlines` |
| `pipeline.rs` | ~750 | `recover_outline`, `recover_outline_seeded`, `tier1_heuristic` |
| `probe.rs` | ~375 | Classify pages as TextBased/ImageOnly/Mixed/Empty |
| `offset.rs` | ~455 | `calculate_offset` (PageLabels → TOC match → page-number scan) |
| `layout.rs` | ~1100 | 4-phase typographic pipeline: extract → profile → features → classify |
| `clustering.rs` | ~410 | Jenks natural-breaks; semantic role assignment (Body/Heading/Footnote) |
| `seed.rs` | ~623 | Reference-based chapter discovery; RANSAC offset voting |
| `page_labels.rs` | ~580 | `/PageLabels` number tree; Roman/alphabetic conversion |
| `text.rs` | ~65 | `extract_all`, `extract_range` (Tantivy indexing input) |

### Chapter detection (3-tier fallback in `engine.rs`)

1. **`/Outlines`** — preferred; walks bookmark tree at configurable depth
2. **`/PageLabels`** — fallback; range transitions become chapter boundaries
3. **Whole document** — final fallback; one chunk = entire PDF

### Tm-matrix scale tracking

Many PDFs scale text via the text matrix (`Tm a b c d e f`) rather than the font descriptor. Arcane computes `tm_scale = √(a² + b²)` so `effective_size = nominal_size × tm_scale`. Applied in `layout.rs` and `heuristics.rs`.

### Layout analysis phases (layout.rs)

- **Phase A** `build_typographic_profile()` — font-size μ, σ, body centroid, gap p90
- **Phase B** `build_text_features()` — z-score, BOLD/ITALIC/ALL_CAPS/ISOLATED flags, case pattern
- **Phase C** `classify_features()` — Bayesian rules → `LayoutAnchor` with confidence (< 0.40 dropped)
- **Boosts** — TOC fuzzy-match +0.20, offset agreement +0.10

## Specialized Sub-Agents (`.claude/agents/`)

Each agent audits a specific module's correctness and efficiency:

| Agent file | Covers |
|------------|--------|
| `pdf-ops-auditor.md` | `ops.rs` — merge, split, rotate, encrypt, inject, extract-pages |
| `layout-analysis-auditor.md` | `layout.rs`, `clustering.rs`, `heuristics.rs` |
| `outline-recovery-auditor.md` | `pipeline.rs`, `seed.rs`, `heuristics.rs` (inject), `outlines.rs` |
| `offset-detection-auditor.md` | `offset.rs`, `page_labels.rs` |
| `bridge-pipeline-auditor.md` | `src/bridge/` — Arcane-PP HTTP client, temp cleanup |
| `search-index-auditor.md` | `src/search/`, `text.rs` — Tantivy indexing |

To invoke an agent, use the Agent tool with `subagent_type: general-purpose` and reference the agent's focus files and responsibilities from the agent file.

## Workflow Skill Files (`.claude/commands/`)

Step-by-step compositions of base commands for common tasks:

| File | Workflow |
|------|----------|
| `chunk-textbook.md` | Chunk a textbook using embedded bookmarks |
| `recover-outline-heuristic.md` | Recover bookmarks via font-size heuristics |
| `recover-outline-seeded.md` | Recover bookmarks using reference PDF or JSON seeds |
| `recover-outline-bridge.md` | Recover bookmarks via OCR of TOC pages (Arcane-PP) |
| `add-and-index.md` | Add a PDF source and index it for search |
| `find-page-offset.md` | Determine physical-to-logical page offset |
| `merge-split-pdf.md` | Merge or split PDFs |
| `encrypt-decrypt-pdf.md` | Protect or unlock a PDF |

## Cross-Platform Notes

- **Symlinks vs copies:** Unix uses symlinks for originals in `Library/`; Windows uses file copies. Abstracted in `storage/filesystem.rs`.
- **Windows paths:** use forward slashes internally; the project builds on Windows, macOS, and Linux.
- Dual store: **SQLite** (primary) + **legacy JSON** (`projects.json`) kept in sync for backward compat.

## Testing Conventions

- Tests live in `#[cfg(test)]` modules at the bottom of each source file.
- Use `tempfile` crate for temporary directories — cleanup is automatic.
- For dry-run chunk boundary testing: `arcane chunk "Project" --dry-run --depth 2`
- Parser unit tests live in `src/cli/commands/helpers.rs`.
