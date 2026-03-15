# Arcane Developer Guide

This guide provides an in-depth look at Arcane's architecture, implementation details, and guidelines for contributing to the project.

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Project Structure](#project-structure)
- [Core Components](#core-components)
- [Design Philosophy](#design-philosophy)
- [Building and Testing](#building-and-testing)
- [Contributing Guidelines](#contributing-guidelines)
- [Future Roadmap](#future-roadmap)

## Architecture Overview

Arcane is designed as a local-first application with a simple three-layer architecture:

```
┌─────────────────────────────────────┐
│         CLI Layer (main.rs)         │  ← Command parsing, user interaction
├─────────────────────────────────────┤
│     Domain Layer (models.rs)        │  ← Core business logic, trait system
├─────────────────────────────────────┤
│   Infrastructure Layer               │
│  - storage.rs  (persistence)        │  ← File system, JSON storage
│  - pdf_engine.rs (PDF processing)   │  ← PDF manipulation, chunking
└─────────────────────────────────────┘
```

### Key Design Decisions

1. **Clap-Based CLI**: Declarative command definitions using `clap` derive macros
2. **Trait-Based Polymorphism**: `Source` trait allows different source types (Textbook, Report) with shared behavior
3. **Dual Storage**: Legacy JSON (`projects.json`) for backward compatibility + SQLite (`arcane.db`) with full relational schema
4. **Content-Addressed Storage (CAS)**: BLAKE3-hashed blob store for deduplication
5. **Idempotent Operations**: Commands like `chunk` can be safely run multiple times
6. **Full-Text Search**: Tantivy-powered search index with project/source filtering
7. **Cross-Platform**: Handles platform differences (Unix symlinks vs Windows copies) transparently

## Project Structure

```
src/
├── main.rs                 # CLI entry point and command dispatch
├── error.rs                # Typed error hierarchy (thiserror)
├── cli/
│   ├── mod.rs              # Clap command definitions
│   ├── commands.rs         # Command handler implementations
│   └── output.rs           # Output formatting helpers
├── models/
│   ├── mod.rs              # Module exports
│   ├── project.rs          # Project struct
│   ├── source.rs           # SourceMeta, Source trait, Textbook/Report
│   ├── chunk.rs            # ChunkRecord struct
│   └── tags.rs             # SourceKind enum
├── pdf/
│   ├── mod.rs              # Module exports
│   ├── engine.rs           # Core chunking engine (boundary detection + parallel writing)
│   ├── heuristics.rs       # Font-size heuristics, heading extraction, outline injection
│   ├── probe.rs            # PDF classification (text-based vs scanned vs mixed)
│   ├── clustering.rs       # Jenks natural-breaks 1D font-size clustering
│   ├── layout.rs           # Position-aware text extraction (text-matrix state machine)
│   ├── offset.rs           # Logical-to-physical page offset calculation
│   ├── pipeline.rs         # Outline recovery pipeline orchestrator
│   ├── outlines.rs         # /Outlines (bookmarks) extraction with depth support
│   ├── page_labels.rs      # /PageLabels parsing and resolution
│   ├── text.rs             # Text extraction from PDF pages
│   ├── ops.rs              # Structural PDF operations (merge, split, rotate, encrypt)
│   └── writer.rs           # Low-level PDF writing helpers
├── search/
│   ├── mod.rs              # Module exports
│   ├── indexer.rs          # Tantivy schema, index lifecycle, document indexing
│   └── query.rs            # Query parsing, search with project/source filters
├── storage/
│   ├── mod.rs              # Module exports
│   ├── database.rs         # SQLite CRUD (projects, sources, tags, blobs)
│   ├── legacy.rs           # Legacy JSON storage (projects.json)
│   ├── cas.rs              # Content-addressed store (BLAKE3 hashing)
│   ├── filesystem.rs       # Directory layout helpers
│   ├── migrations.rs       # Schema migration runner
│   └── sql/
│       ├── v1_core.sql     # Core schema (projects, sources, tags, blobs, chunks)
│       └── v2_add_depth_pagecount.sql  # Adds depth and page_count columns to sources
├── ui/
│   ├── mod.rs              # TUI module
│   ├── app.rs              # Application state machine
│   └── event.rs            # Key event handling
└── watcher/
    ├── mod.rs              # File watcher module
    └── handlers.rs         # Watch event classification
```

### Dependencies

**Production Dependencies:**
- `lopdf` (0.39.0) - PDF manipulation without quality loss
- `rayon` (1.10) - Parallel chunk writing
- `tantivy` (0.22) - Full-text search engine
- `rusqlite` (0.33) - SQLite database
- `clap` (4.5) - CLI argument parsing with derive macros
- `serde` (1.0) + `serde_json` (1.0) - Serialization
- `anyhow` (1.0) + `thiserror` (2) - Error handling
- `blake3` (1) - Content-addressed hashing
- `strsim` (0.11) - Fuzzy string matching (Levenshtein, Jaro-Winkler) for outline verification
- `chrono` (0.4) - Timestamps
- `uuid` (1) - Unique IDs
- `tracing` + `tracing-subscriber` - Structured logging
- `notify` (7) - File system watching
- `ratatui` (0.29) - Terminal UI framework
- `crossterm` (0.28) - Terminal manipulation

**Development Dependencies:**
- `tempfile` (3) - Temporary directories for testing

## Core Components

### 1. CLI Layer (`src/cli/`)

The CLI layer uses `clap` derive macros for declarative command definitions in `mod.rs` and implements handlers in `commands.rs`.

**Key Command Handlers (commands.rs):**
- `cmd_new()`: Creates a new project
- `cmd_list()`: Lists all projects and sources
- `cmd_show()`: Shows detailed project information
- `cmd_add()`: Adds a PDF source (CAS ingest + dual-store persistence)
- `cmd_chunk()`: Splits textbooks into chapters (supports `--force`, `--depth`, `--dry-run`, `--source` for per-textbook depth)
- `cmd_search()`: Full-text search with optional project/source filters
- `cmd_remove()`: Removes a source or entire project (cascading cleanup)
- `cmd_list_chunks()`: Lists chunk files for a source
- `cmd_outline()`: Displays PDF outline tree and page labels
- `cmd_probe()`: Classifies a PDF as text-based, scanned, or mixed
- `cmd_detect_layout()`: Detects structural anchors (headings, TOC entries, page numbers)
- `cmd_find_offset()`: Calculates logical-to-physical page offset
- `cmd_recover_outline()`: Full outline recovery pipeline (probe → cluster → extract → offset → verify → inject)
- `cmd_reindex()`: Rebuilds the search index
- `cmd_tag()` / `cmd_untag()`: Tag management
- `cmd_merge()` / `cmd_split()` / `cmd_rotate()`: Structural PDF operations
- `cmd_protect()` / `cmd_unlock()`: PDF encryption
- `cmd_watch()`: File system watcher
- `cmd_tui()`: Interactive terminal UI

**Example Command Flow:**
```
arcane add "Project" book.pdf --textbook --start-page 12
     ↓
cmd_add() via clap dispatch
     ↓
CAS ingest (BLAKE3 hash + dedup)
     ↓
Symlink/copy in Originals/
     ↓
Legacy JSON store + SQLite database
```

### 2. Domain Layer (`models.rs`)

The domain layer defines the core abstractions and business logic.

#### Key Types

**`Project`** (src/models.rs:20)
```rust
pub struct Project {
    pub name: String,           // Unique project identifier
    pub tags: Vec<String>,      // Optional organizational tags
    pub sources: Vec<SourceMeta>, // All PDF sources in this project
}
```

**`SourceMeta`** (src/models/source.rs)
```rust
pub struct SourceMeta {
    pub title: String,                      // Display name
    pub path: PathBuf,                      // Path to original PDF
    pub needs_chunking: bool,               // Textbook vs Report
    pub chapter_map: HashMap<u32, String>,  // Physical page → chapter name
    pub start_page_physical: Option<u32>,   // Offset for page numbering
    pub depth: Option<u32>,                 // Outline depth used for last chunk
    pub page_count: Option<u32>,            // Total pages in the PDF
}
```

**`Source` trait** (src/models/source.rs)
```rust
pub trait Source {
    fn chunk(&self, chunks_dir: &Path, depth: u32) -> Result<(u32, usize)>;
    fn youtube(&self, url: &str) -> Result<()>; // Future feature
}
```

The `depth` parameter controls how many levels of the PDF outline tree to walk (1 = top-level chapters, 2+ = sub-sections). Returns `(page_count, chunk_count)` on success.

#### Trait Implementations

- **`Textbook`**: Large PDFs that need chapter splitting — calls `chunk_pdf()` with depth
- **`Report`**: Small PDFs stored as-is without chunking — `chunk()` is a no-op

The `build_source()` factory function dispatches to the correct implementation based on `needs_chunking`.

### 3. PDF Engine (`src/pdf/`)

The PDF engine handles intelligent chapter detection and PDF splitting across multiple modules.

#### Chapter Detection Strategy (the "Xodo Method")

The engine tries three methods in order of preference (see `resolve_chapters()` in `engine.rs`):

1. **`/Outlines` (Bookmarks/TOC)** - Most reliable (`outlines.rs`)
   - Walks the PDF outline tree with configurable depth
   - Depth 1 = top-level chapters only (siblings via `Next`)
   - Depth 2+ = recurses into children via `First` for sub-sections
   - Nested titles formatted as `"Parent > Child"`
   - Resolves direct destinations, indirect references, and named destinations

2. **`/PageLabels` Dictionary** - Fallback (`page_labels.rs`)
   - Parses the PageLabels number tree
   - Supports Roman (`/r`, `/R`), Arabic (`/D`), and alphabetic (`/a`, `/A`) styles
   - Uses range transitions as chapter boundaries

3. **Whole-document fallback** - Treats the entire PDF as one chunk

#### Physical vs Logical Page Mapping

Many textbooks have front matter with Roman numerals (i, ii, iii, ...) before the actual content begins at "Page 1". The PDF engine handles this through an offset calculation:

```
offset = physical_index - logical_page_number
```

#### Key Functions

- **`chunk_pdf(meta, chunks_dir, depth)`** (`engine.rs`): Main entry point for chunking
- **`detect_boundaries(meta, depth)`** (`engine.rs`): Detect boundaries without writing (for `--dry-run`)
- **`resolve_chapters(meta, doc, depth)`** (`engine.rs`): Unified chapter detection strategy
- **`extract_chapters_with_depth(doc, depth)`** (`outlines.rs`): Depth-aware outline walker
- **`extract_chapters_from_page_labels(doc)`** (`page_labels.rs`): Page label parsing
- **`boundaries_to_ranges(chapters, total)`** (`engine.rs`): Convert boundaries to page ranges
- **`write_chunk(doc, start, end, path)`** (`engine.rs`): BFS-based minimal page extraction
- **`sanitise_filename(name)`** (`engine.rs`): Clean chapter titles for filenames

#### Outline Recovery Pipeline

When a PDF has no `/Outlines` bookmarks, the `recover-outline` command runs a modular pipeline to reconstruct them:

```
pipeline.rs (orchestrator)
    │
    ├── probe.rs         — Classify pages as Text/Image/Mixed/Empty
    ├── clustering.rs    — Jenks natural-breaks on font-size histogram
    ├── layout.rs        — Text-matrix state machine for (x, y) extraction
    │       │
    │       └── Detects: ChapterHeading, SectionHeading, TocEntry, PageNumber
    ├── offset.rs        — Logical-to-physical page delta
    │       │
    │       ├── Strategy 1: /PageLabels number tree
    │       ├── Strategy 2: TOC matching (fuzzy, via strsim)
    │       └── Strategy 3: Header/footer page-number consensus
    └── heuristics.rs    — Heading extraction + outline injection
            │
            ├── inject_outlines()                — Flat /Outlines tree
            └── inject_hierarchical_outlines()   — Nested Chapter > Section tree
```

**Key types:**
- `ProbeResult` — document classification with per-page breakdown
- `FontCluster` / `FontRole` — clustered font sizes with semantic roles (Body, Heading1, Heading2, Footnote)
- `PositionedText` — text run with (x, y, font_size) from the text-matrix state machine
- `LayoutAnchor` / `AnchorKind` — structural element detected by position + font analysis
- `OffsetResult` / `OffsetMethod` — page offset with confidence score and evidence chain
- `RecoveryConfig` — pipeline configuration (min_font_ratio, depth, toc_pages, fuzzy_threshold)
- `RecoveryResult` — full pipeline output (probe, layout, offset, headings, verification, injection count)

The pipeline is designed so each module can be used independently via CLI sub-commands (`probe`, `detect-layout`, `find-offset`) or composed together for the full `recover-outline` flow.

#### Performance

- **BFS object traversal**: `write_chunk` only copies objects reachable from target pages (not the full document)
- **`rayon` parallel writes**: Each chunk is written on a separate thread
- **Page map built once**: `oid_to_page` map avoids O(N × P) per-entry lookups
- **Detection-only passes**: `probe` and `detect-layout` read content streams without modifying the document
- **Selective processing**: `--toc-pages` flag skips TOC discovery and targets specific pages directly

#### Idempotency

The `is_already_chunked()` function checks if the chunks directory already contains PDF files. If so, `chunk_pdf()` returns early without reprocessing. Use `--force` to bypass this guard.

### 4. Storage Layer (`src/storage/`)

The storage layer manages persistence across three backends: legacy JSON, SQLite, and CAS.

#### File System Layout

```
~/Arcane/
├── projects.json                    # Legacy JSON store (backward compat)
├── arcane.db                        # SQLite database
├── CAS/                             # Content-addressed blob store
│   └── {prefix}/{hash}/blob
├── search_index/                    # Tantivy full-text search index
└── Library/
    └── [Project_Name]/
        ├── Originals/               # Symlinks/copies to original PDFs
        └── Chunks/
            └── [Source_Title]/      # Per-source chunk PDFs
```

#### SQLite Database (`database.rs`)

The `Database` struct wraps a `rusqlite::Connection` with WAL mode and foreign keys enabled.

**Key methods:**
- `create_project()` / `delete_project()`: Project CRUD
- `add_source()` / `delete_source()`: Source CRUD (FK cascades handle tags, chunks, search_meta)
- `get_sources()` / `get_source_titles()`: Source queries
- `add_project_tag()` / `remove_project_tag()`: Tag management
- `add_source_tag()` / `get_source_tags()`: Source-level tags
- `register_blob()` / `blob_exists()` / `get_blob_path()`: CAS blob tracking

Schema is managed via versioned migrations (`migrations.rs`, `sql/v1_core.sql`). Foreign keys use `ON DELETE CASCADE` for automatic cleanup.

#### Legacy JSON Store (`legacy.rs`)

The `ProjectStore` provides backward-compatible CRUD for `projects.json`:
- `load()` / `save()`: Persistence
- `get()` / `get_mut()`: Query by name
- `upsert()` / `remove()`: Insert/update/delete

A migration helper (`migrate_if_needed()`) can import JSON data into SQLite.

#### Content-Addressed Store (`cas.rs`)

Uses BLAKE3 hashing for deduplication. Layout: `~/Arcane/CAS/{first-2-hex}/{full-hash}/blob`.
- `ingest(path)`: Hash file, store if new, return `BlobRef`
- `hash_file(path)`: Compute hash without storing
- `resolve(hash)`: Check if blob exists

#### Filesystem Helpers (`filesystem.rs`)

- `arcane_root()`: Returns `~/Arcane/`
- `project_dir()` / `originals_dir()` / `chunks_dir()`: Directory paths
- `source_chunks_dir(project, title)`: Per-source chunk directory
- `link_original_to_cas()`: Symlink on Unix, copy on Windows

### 5. Search Layer (`src/search/`)

Tantivy-powered full-text search with six fields: `source_id`, `project`, `title`, `chapter`, `page`, `body`.

**Indexer (`indexer.rs`):**
- `index_source()`: Index a source's extracted pages
- `remove_source()`: Remove all documents for a source ID

**Query (`query.rs`):**
- `search()`: Full-text query with optional `project_filter` and `source_filter`
- Filters are implemented as `BooleanQuery` with `TermQuery` clauses on STRING fields

## Design Philosophy

### 1. Lean Dependencies

Arcane uses well-established crates (`clap`, `tantivy`, `rusqlite`, `lopdf`) but avoids unnecessary abstractions.

### 2. Local-First

All data is stored locally in `~/Arcane/`. No cloud services, no network dependencies, no telemetry. Users have complete control over their data.

### 3. Idempotent Operations

Commands are designed to be safely re-runnable:
- `chunk` won't reprocess existing chapters (unless `--force` is used)
- `add` won't duplicate sources (CAS deduplication)
- `new` won't overwrite existing projects

### 4. Lossless PDF Processing

The PDF engine uses `lopdf`'s extraction facilities without re-encoding streams. This preserves the original quality and keeps operations fast.

### 5. Cross-Platform

The application handles platform differences transparently:
- Unix: Uses symlinks for efficiency
- Windows: Falls back to file copies
- Path handling: Uses `PathBuf` consistently

## Building and Testing

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Install locally
cargo install --path .
```

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

### Test Structure

Tests are organized as inline `#[cfg(test)]` modules at the end of each source file. Current test suite: **79 tests**.

Key test areas:
- `models/project.rs`: Project operations
- `models/source.rs`: Source metadata serialization, build_source dispatch
- `pdf/engine.rs`: Boundary calculation, filename sanitization, idempotency
- `pdf/heuristics.rs`: Font-size histogram, heading extraction, chapter map deduplication
- `pdf/probe.rs`: Page classification (text/image/empty), document-level probe, per-page counts
- `pdf/clustering.rs`: Jenks clustering, role assignment (body/heading/footnote), edge cases
- `pdf/layout.rs`: Text-matrix tracking, TOC entry patterns, page-number detection, anchor detection
- `pdf/offset.rs`: TOC line parsing, fuzzy similarity, page-number consensus, range filtering
- `pdf/pipeline.rs`: Heading merging, chapter map building, line similarity, HeadingInfo conversion
- `pdf/page_labels.rs`: Roman numeral conversion, label resolution
- `pdf/ops.rs`: Input validation for merge, split, rotate, encrypt
- `search/indexer.rs`: Index creation, page indexing
- `search/query.rs`: Full-text search, cross-source search
- `storage/database.rs`: Full CRUD coverage (projects, sources, tags, blobs)
- `storage/legacy.rs`: JSON round-trip, upsert/remove
- `storage/cas.rs`: Ingest, dedup, hash determinism
- `ui/app.rs`: State machine transitions
- `watcher/handlers.rs`: Event classification

Tests use `tempfile` for creating temporary directories that are automatically cleaned up.

## Contributing Guidelines

### Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/Arcane.git`
3. Create a feature branch: `git checkout -b feature/my-feature`
4. Make your changes
5. Run tests: `cargo test`
6. Run formatter: `cargo fmt`
7. Run linter: `cargo clippy`
8. Commit with clear messages
9. Push and create a pull request

### Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Address all `clippy` warnings (`cargo clippy`)
- Write doc comments for public APIs using `///`
- Use descriptive variable names
- Keep functions focused and small

### Documentation

- Update relevant documentation when changing functionality
- Add doc comments to new public functions
- Update USER_GUIDE.md for user-facing changes
- Update DEVELOPER_GUIDE.md for architectural changes

### Testing

- Add tests for new functionality
- Ensure all existing tests pass
- Use `tempfile` for tests that need file system access
- Test edge cases and error conditions

### Commit Messages

Use clear, descriptive commit messages:

```
Good:
- "Add support for custom chapter titles in PDF metadata"
- "Fix panic when processing PDFs without outlines"
- "Improve error messages for missing projects"

Bad:
- "Fix bug"
- "Update code"
- "Changes"
```

## Future Roadmap

### Planned Features

1. **OCR Support for Scanned PDFs** (Tier 2 Outline Recovery)
   - See [OCR Integration Plan](#ocr-integration-plan) below for full details
   - Extract text from scanned/image-only PDFs
   - Make scanned textbooks searchable and chunkable

2. **YouTube Integration** (src/models/source.rs — `youtube()` trait method)
   - Extract transcripts from lecture videos
   - Link video timestamps to textbook sections
   - Store as markdown with video embeds

3. **CAS Garbage Collection** (`gc` command)
   - Find orphaned blobs not referenced by any source
   - Reclaim disk space after source/project removal

4. **Integrity Checking** (`doctor` command)
   - Verify every DB source has its filesystem counterpart
   - Re-hash CAS blobs to detect corruption

5. **Export Functionality**
   - Export project metadata and PDFs to a portable directory
   - Bundle for sharing or migration

6. **GUI Interface**
   - Tauri-based GUI with visual project organization
   - PDF preview in app

### OCR Integration Plan

The outline recovery pipeline currently handles text-based PDFs (Tier 1). For scanned PDFs, `arcane probe` reports `Scanned (Image-Only)` and the pipeline gracefully reports that OCR is required. This section describes the planned Tier 2 implementation.

#### Architecture

OCR will be an **optional feature flag** to keep the default build zero-C-dependency:

```toml
[features]
default = []
ocr = ["dep:ocrs", "dep:image", "dep:pdfium-render"]

[dependencies.ocrs]
version = "0.11"
optional = true

[dependencies.image]
version = "0.25"
optional = true

[dependencies.pdfium-render]
version = "0.8"
optional = true
features = ["image"]
```

Build with OCR support:
```bash
cargo build --release --features ocr
```

#### Crate Selection

| Crate | Role | Why |
|-------|------|-----|
| **`ocrs`** (v0.11) | OCR engine | Pure-Rust ONNX inference via RTen. Returns word-level bounding boxes with text. No C dependencies. Latin alphabet support sufficient for English TOC entries. |
| **`rten`** | ONNX runtime | Transitive dependency via `ocrs`. Pure Rust. |
| **`image`** (v0.25) | Image buffers | Standard `ImageBuffer` types for OCR input. |
| **`pdfium-render`** (v0.8) | PDF → image | Renders PDF pages to raster images for OCR input. Uses Google's PDFium via FFI — the only C dependency, accepted because native PDF rasterisation has no pure-Rust equivalent. |

Alternatives considered and rejected:
- **`ocr-rs`**: Uses PaddleOCR via MNN inference framework. Viable but less mature than `ocrs` in the Rust ecosystem.
- **`tesseract-rs`**: Wraps Tesseract via C FFI. Heavier dependency chain, requires system-level Tesseract installation.

#### Module Design

A new `src/pdf/ocr.rs` module behind `#[cfg(feature = "ocr")]`:

```rust
/// Render a single PDF page to an image at the given DPI.
#[cfg(feature = "ocr")]
pub fn render_page_to_image(path: &Path, page_index: u32, dpi: u32) -> Result<DynamicImage>;

/// Run OCR on a rendered page image, returning text with bounding boxes.
#[cfg(feature = "ocr")]
pub fn ocr_page(image: &DynamicImage) -> Result<Vec<OcrTextBlock>>;

/// Selectively OCR only the TOC pages and page-corner regions.
/// Returns positioned text suitable for the layout/offset pipeline.
#[cfg(feature = "ocr")]
pub fn ocr_toc_pages(
    path: &Path,
    toc_pages: (u32, u32),
    corner_pages: &[u32],
) -> Result<Vec<PositionedText>>;
```

```rust
/// A block of text recognized by OCR with its bounding box.
pub struct OcrTextBlock {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
}
```

#### Pipeline Integration

The `pipeline.rs` orchestrator will route scanned PDFs through Tier 2:

```
probe() → DocumentKind::Scanned
    │
    ├── If --toc-pages provided:
    │       ocr_toc_pages(path, toc_range, sample_pages)
    │       → PositionedText (same type as Tier 1)
    │       → offset::parse_toc_entries() + offset::calculate_offset()
    │       → inject outlines
    │
    └── If no --toc-pages:
            1. Render first 20 pages at 150 DPI
            2. Run OCR detection-only pass (bounding boxes, no recognition)
            3. Use layout heuristics (text density, centering, font height)
               to identify candidate TOC pages
            4. OCR only those candidate pages + page corners
            5. Continue with offset → verify → inject
```

**Key design decisions:**
- **Selective OCR**: Never OCR the entire document. Only process TOC pages and page-corner regions (for page-number detection). This keeps the operation fast even for 1000+ page textbooks.
- **Detection-only first pass**: Use `ocrs` detection model (bounding boxes without text recognition) to find text-dense regions. This is ~5x faster than full recognition and sufficient for identifying TOC page candidates.
- **Quantized models**: Use FP16 ONNX models for ~2x inference speedup with minimal accuracy loss on printed text.
- **`--toc-pages` bypass**: When the user provides `--toc-pages`, skip all discovery and target those pages directly. This is the fastest and most reliable path.

#### Boldness Estimation for Scanned Headings

For scanned PDFs, font-size information isn't available from content streams. Instead, heading detection will use the `imageproc` crate for:

1. **Stroke width estimation**: Measure the average stroke width of text in bounding boxes. Bold text has wider strokes.
2. **Height-based size classification**: Use the bounding box height as a proxy for font size. Cluster these heights using the same Jenks algorithm from `clustering.rs`.
3. **Centering heuristics**: Text centered on the page (x-position near page midpoint) with large bounding boxes is likely a heading.

This would require adding `imageproc` as an optional dependency under the `ocr` feature flag.

#### Error Handling

A new `OcrError` variant will be added to `PdfError` in `src/error.rs`:

```rust
#[cfg(feature = "ocr")]
#[error("OCR error: {0}")]
Ocr(String),
```

When OCR is not compiled in (`default` features), `pipeline.rs` returns a clear message:
```
[arcane] Scanned PDF detected — OCR support not available.
         Rebuild with: cargo build --release --features ocr
```

### Contributing to Roadmap

Have an idea for a new feature? Open an issue on GitHub to discuss it!

## Technical Debt and Known Limitations

### Current Limitations

1. **Limited Error Recovery**: If a PDF is malformed, the entire chunking operation may fail

2. **No Progress Indication**: Large PDFs don't show progress during chunking

3. **Chapter Name Accuracy**: Depends entirely on PDF metadata quality

4. **No CAS Cleanup**: Removing sources/projects does not delete CAS blobs (by design — shared references)

5. **Dual-Store Sync**: Both JSON and SQLite stores must be kept in sync during writes

### Areas for Improvement

1. **Progress Bars**: Add progress indication for long operations

2. **Batch Operations**: Support for adding multiple PDFs at once

3. **Configuration File**: Allow users to customize default paths and behavior

4. **Source Editing**: Commands to rename or move sources between projects

## Debugging Tips

### Common Issues

**Build Failures:**
```bash
# Clean and rebuild
cargo clean
cargo build
```

**Test Failures:**
```bash
# Run tests with verbose output
cargo test -- --nocapture

# Run a specific test
cargo test test_name -- --nocapture
```

**PDF Processing Issues:**

Enable verbose logging via the `RUST_LOG` environment variable:
```bash
RUST_LOG=arcane=debug arcane chunk "Project"
```

Or inspect a PDF's structure directly:
```bash
arcane outline problem-file.pdf --depth 5
```

### Useful Commands

```bash
# Check for warnings
cargo clippy

# Format code
cargo fmt

# Generate documentation
cargo doc --open

# Check dependencies
cargo tree
```

## Getting Help

- **Issues**: Report bugs at https://github.com/Rah-Rah-Mitra/Arcane/issues
- **Discussions**: Ask questions in GitHub Discussions
- **Pull Requests**: Submit improvements via pull requests

## License

Arcane is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
