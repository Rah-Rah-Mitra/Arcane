# Arcane

A local-first research archival application for organizing academic materials. Arcane helps researchers and students manage PDF documents — textbooks, research papers, lecture notes — with automatic chapter splitting and full-text search.

## Features

- **Project-Based Organization**: Group related sources under named projects with optional tags
- **Smart PDF Chunking**: Automatically split textbooks into individual chapter files
- **Multi-Level Chapter Detection**: Extracts chapter boundaries from PDF bookmarks/outlines with configurable depth (sub-chapters, sections)
- **Outline Recovery Pipeline**: Reconstruct missing bookmarks using a multi-heuristic structural classifier — statistical typographic profiling (μ/σ/Z-scores), bold/italic/case feature extraction, Bayesian confidence scoring, and fuzzy verification — then inject them back as functional PDF outlines
- **Page Sync / RANSAC Offset**: Correlate detected headings with TOC entries using RANSAC-style consensus voting to find the physical-to-logical page offset
- **PDF Classification**: Instantly determine whether a PDF is text-based, scanned, or mixed
- **Page Offset Detection**: Automatically calculate the front-matter delta between printed and physical page numbers
- **Physical/Logical Page Mapping**: Correctly handles front-matter with Roman numerals
- **Deduplication**: Content-addressed storage (CAS) — adding the same file twice is a no-op
- **Full-Text Search**: tantivy-powered search across all indexed sources, with project and source filters
- **PDF Inspection**: View outline trees and page labels of any PDF before adding
- **Local-First**: All data stored in `~/Arcane/` with no cloud dependency
- **Cross-Platform**: Works on Windows (file copies) and Unix (symlinks)

## Quick Start

### Build

```bash
git clone https://github.com/Rah-Rah-Mitra/Arcane.git
cd Arcane
cargo build --release          # always use --release for PDF-heavy workloads
```

### Workflow

```bash
# 1. Create a project
arcane new "Rust-Programming"

# 2. Add sources  (relative or absolute paths work)
arcane add "Rust-Programming" path/to/textbook.pdf --textbook
arcane add "Rust-Programming" path/to/paper.pdf              # report (no chunking)

# 3. Inspect the PDF's outline before chunking
arcane outline path/to/textbook.pdf --depth 3

# 4. Preview chunk boundaries without writing files
arcane chunk "Rust-Programming" --dry-run --depth 2

# 5. Split textbooks into per-chapter PDFs (use --release build for speed)
arcane chunk "Rust-Programming"
# Or chunk a specific textbook at its own depth:
arcane chunk "Rust-Programming" --source "The Rust Book" --depth 2

# 6. Inspect
arcane list
arcane show "Rust-Programming"
arcane list-chunks "Rust-Programming"

# 7. Search (optionally filter by project or source)
arcane search "ownership lifetimes" --limit 10
arcane search "borrowing" --project "Rust-Programming"

# 8. Remove a source or entire project
arcane remove "Rust-Programming" "textbook"
arcane remove "Rust-Programming"               # removes entire project
```

> **Performance note**: always run `cargo build --release` and use the release binary
> (`./target/release/arcane`) for the `chunk` command. Debug builds are 10–100× slower
> for CPU-bound PDF parsing. An unoptimized build can take minutes; a release build
> finishes the same work in under a second.

## Chapter Detection

When you run `chunk`, Arcane tries three strategies in order:

1. **PDF Outlines / Bookmarks** (`/Outlines` tree) — preferred. Resolves both direct
   array destinations and named destinations from the `/Names/Dests` tree.
   Use `--depth N` to control how many levels of the outline tree to use
   (1 = top-level chapters only, 2 = chapters + sections, etc.).
2. **Page Labels** (`/PageLabels`) — fallback when no usable bookmarks exist.
3. **Whole-document fallback** — treats the entire PDF as one chunk.

If a PDF has no bookmarks at all, use `arcane recover-outline` to reconstruct them
before chunking. You can also use `arcane probe` to classify a PDF and
`arcane detect-layout` to inspect its structural anchors.

Use `arcane outline <file>` to inspect a PDF's outline tree and page labels before chunking.
Use `arcane chunk <project> --dry-run` to preview detected boundaries without writing files.

Chapter PDFs are written to `~/Arcane/Library/<Project>/Chunks/<Source>/`.

## Storage Layout

```
~/Arcane/
├── projects.json              # project + source metadata
├── arcane.db                  # SQLite (tags, blobs, search index metadata)
├── CAS/                       # content-addressed blob store
└── Library/
    └── <Project>/
        ├── Originals/         # symlinks (Unix) or copies (Windows) of source PDFs
        └── Chunks/
            └── <Source>/      # per-chapter PDFs: d1_01_Introduction.pdf, …
```

## All Commands

| Command | Description |
|---------|-------------|
| `arcane new <name>` | Create a new project |
| `arcane list` | List all projects and their sources |
| `arcane show <name>` | Show detailed project info |
| `arcane tag <project> <tag>` | Add a tag to a project |
| `arcane untag <project> <tag>` | Remove a tag from a project |
| `arcane add <project> <path> [--textbook] [--start-page N] [--title "…"] [--tag TAG]… [--type TYPE]` | Add a source PDF. `--tag` is repeatable. `TYPE`: textbook \| report \| paper \| cheatsheet \| custom string |
| `arcane chunk <project> [--force] [--depth N] [--dry-run] [--source S]` | Split textbooks into per-chapter PDFs. `--depth` default: 1 (1 = top-level chapters, 2+ = sub-sections) |
| `arcane list-chunks <project> [source]` | List chunk files for a source (or all sources if omitted) |
| `arcane search <query> [--limit N] [--project P] [--source S]` | Full-text search across all indexed sources. `--limit` default: 10 |
| `arcane reindex` | Rebuild the full-text search index from all sources |
| `arcane outline <file> [--depth N]` | Show PDF outline tree and page labels. `--depth` default: 10 |
| `arcane probe <file> [--json]` | Classify PDF as text-based, scanned, or mixed |
| `arcane detect-layout <file> [--json] [--pages RANGE]` | Detect structural anchors via statistical typographic profiling. `--pages`: 0-based range (e.g. "0-5") |
| `arcane find-offset <file> [--toc-pages RANGE] [--json]` | Calculate logical-to-physical page offset. `--toc-pages`: 1-based (e.g. "3-5") |
| `arcane sync-pages <file> [--toc-pages RANGE] [--threshold T] [--json]` | RANSAC consensus offset from heading↔TOC matching. `--threshold` default: 0.6 (range: 0.0–1.0). `--toc-pages`: 1-based |
| `arcane recover-outline <file> [--output PATH] [--dry-run] [--min-font-ratio R] [--depth N] [--toc-pages RANGE] [--no-inject] [--fuzzy-threshold T] [--json] [--seed-pdf PDF] [--seed-file JSON] [--seed-tolerance N]` | Recover and inject outline bookmarks. `--min-font-ratio` default: 1.2. `--depth` default: 2 (1 = chapters, 2 = +sections). `--fuzzy-threshold` default: 0.6 (0.0–1.0). `--seed-tolerance` default: 5. `--seed-pdf` and `--seed-file` are mutually exclusive |
| `arcane ocr <file> --pages RANGE [--dpi N] [--json]` | Run OCR on a page range. `--pages` required, 1-based (e.g. "1-5"). `--dpi` default: 150. Requires `--features ocr` build + `arcane init-ocr` |
| `arcane init-ocr [--models-dir DIR] [--skip-runtime] [--force]` | Download OCR models and runtime libraries to ~/Arcane/models/ |
| `arcane remove <project> [source]` | Remove a source or entire project (if source omitted) |
| `arcane merge <output> <inputs…>` | Merge multiple PDF files into one |
| `arcane split <input> <ranges…> [--output-dir DIR]` | Split a PDF by page ranges. `--output-dir` default: current directory. Ranges: 1-based, inclusive (e.g. "1-5" "6-10") |
| `arcane rotate <input> [--degrees N] [--output PATH] [--pages P…]` | Rotate PDF pages. `--degrees` default: 90 (must be multiple of 90). `--pages`: 0-based indices; if omitted, all pages rotated |
| `arcane protect <input> --password P [--output PATH]` | Encrypt a PDF with a password. Overwrites input if `--output` omitted |
| `arcane unlock <input> --password P [--output PATH]` | Decrypt a password-protected PDF. Overwrites input if `--output` omitted |
| `arcane watch <project>` | Watch a project directory for new PDFs |
| `arcane tui` | Launch the interactive terminal UI |

## OCR Support (Optional)

For PDFs with broken font encoding (garbled text extraction), Arcane includes an
optional OCR overlay tier using PaddleOCR v5 via ONNX Runtime:

```bash
# Build with OCR
cargo build --release --features ocr

# Download models + runtime libraries (one-time, all platforms)
arcane init-ocr

# recover-outline now handles encoding-broken PDFs
arcane recover-outline "book.pdf" --toc-pages 14-20 --output "book-fixed.pdf"
```

`init-ocr` downloads everything to `~/Arcane/models/` and auto-detects at runtime.
Use `--force` to re-download, `--skip-runtime` to skip ONNX Runtime and PDFium DLLs.

For non-English PDFs, override the recognition model and dictionary via env vars:
`ARCANE_OCR_REC_MODEL`, `ARCANE_OCR_DICT` (see `src/pdf/ocr.rs` for details).

## Requirements

- Rust 1.75 or later
- Cargo

## License

MIT License — see [LICENSE](LICENSE) for details.

## Contributing

See [DEVELOPER_GUIDE.md](DEVELOPER_GUIDE.md) for architecture overview and contribution guidelines.
