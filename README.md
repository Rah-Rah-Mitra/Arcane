# Arcane

A local-first research archival application for organizing academic materials. Arcane helps researchers and students manage PDF documents — textbooks, research papers, lecture notes — with automatic chapter splitting and full-text search.

## Features

- **Project-Based Organization**: Group related sources under named projects with optional tags
- **Smart PDF Chunking**: Automatically split textbooks into individual chapter files
- **Intelligent Chapter Detection**: Extracts chapter boundaries from PDF bookmarks/outlines (with named-destination support) or page labels
- **Physical/Logical Page Mapping**: Correctly handles front-matter with Roman numerals
- **Deduplication**: Content-addressed storage (CAS) — adding the same file twice is a no-op
- **Full-Text Search**: tantivy-powered search across all indexed sources
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

# 3. Split textbooks into per-chapter PDFs (use --release build for speed)
arcane chunk "Rust-Programming"

# 4. Inspect
arcane list
arcane show "Rust-Programming"

# 5. Search
arcane search "ownership lifetimes" --limit 10
```

> **Performance note**: always run `cargo build --release` and use the release binary
> (`./target/release/arcane`) for the `chunk` command. Debug builds are 10–100× slower
> for CPU-bound PDF parsing. An unoptimized build can take minutes; a release build
> finishes the same work in under a second.

## Chapter Detection

When you run `chunk`, Arcane tries three strategies in order:

1. **PDF Outlines / Bookmarks** (`/Outlines` tree) — preferred. Resolves both direct
   array destinations and named destinations from the `/Names/Dests` tree.
2. **Page Labels** (`/PageLabels`) — fallback when no usable bookmarks exist.
3. **Whole-document fallback** — treats the entire PDF as one chunk.

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
            └── <Source>/      # per-chapter PDFs: 01_Introduction.pdf, …
```

## All Commands

| Command | Description |
|---------|-------------|
| `arcane new <project>` | Create a new project |
| `arcane add <project> <file> [--textbook] [--start-page N] [--title "…"]` | Add a source |
| `arcane chunk <project>` | Split textbooks into chapter PDFs |
| `arcane list` | List all projects and their sources |
| `arcane show <project>` | Show detailed project info |
| `arcane search <query> [--limit N]` | Full-text search |
| `arcane reindex <project>` | Rebuild the search index for a project |
| `arcane tag <project> <tag>` | Add a tag to a project |
| `arcane untag <project> <tag>` | Remove a tag |
| `arcane protect <project> <file> <password>` | Password-protect a PDF |
| `arcane unlock <project> <file> <password>` | Remove password protection |
| `arcane tui` | Launch the interactive terminal UI |

## Requirements

- Rust 1.75 or later
- Cargo

## License

MIT License — see [LICENSE](LICENSE) for details.

## Contributing

See [DEVELOPER_GUIDE.md](DEVELOPER_GUIDE.md) for architecture overview and contribution guidelines.
