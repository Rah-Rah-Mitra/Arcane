# Arcane

A local-first research archival application for organizing academic materials. Arcane helps researchers and students manage PDF documents — textbooks, research papers, lecture notes — with automatic chapter splitting and full-text search.

## Features

- **Project-Based Organization**: Group related sources under named projects with optional tags
- **Smart PDF Chunking**: Automatically split textbooks into individual chapter files
- **Multi-Level Chapter Detection**: Extracts chapter boundaries from PDF bookmarks/outlines with configurable depth (sub-chapters, sections)
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
| `arcane new <project>` | Create a new project |
| `arcane add <project> <file> [--textbook] [--start-page N] [--title "…"] [--tag T] [--type T]` | Add a source |
| `arcane chunk <project> [--force] [--depth N] [--dry-run] [--source S]` | Split textbooks into chapter PDFs |
| `arcane list` | List all projects and their sources |
| `arcane list-chunks <project> [source]` | List chunk files for a source |
| `arcane show <project>` | Show detailed project info |
| `arcane search <query> [--limit N] [--project P] [--source S]` | Full-text search with optional filters |
| `arcane reindex` | Rebuild the full-text search index |
| `arcane tag <project> <tag>` | Add a tag to a project |
| `arcane untag <project> <tag>` | Remove a tag |
| `arcane outline <file> [--depth N]` | Show PDF outline tree and page labels |
| `arcane recover-outline <file> [--output F] [--dry-run] [--min-font-ratio R] [--depth N]` | Re-hydrate outlines via font-size heuristics (for PDFs with no bookmarks) |
| `arcane remove <project> [source]` | Remove a source or entire project |
| `arcane merge <output> <inputs…>` | Merge multiple PDFs into one |
| `arcane split <input> <ranges…> [--output-dir D]` | Split a PDF by page ranges |
| `arcane rotate <input> [--degrees N] [--pages P]` | Rotate PDF pages |
| `arcane protect <input> --password P` | Password-protect a PDF |
| `arcane unlock <input> --password P` | Remove password protection |
| `arcane watch <project>` | Watch project directory for new PDFs |
| `arcane tui` | Launch the interactive terminal UI |

## Requirements

- Rust 1.75 or later
- Cargo

## License

MIT License — see [LICENSE](LICENSE) for details.

## Contributing

See [DEVELOPER_GUIDE.md](DEVELOPER_GUIDE.md) for architecture overview and contribution guidelines.
