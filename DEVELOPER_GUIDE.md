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

1. **Zero External CLI Framework**: Hand-rolled argument parser to keep dependencies minimal
2. **Trait-Based Polymorphism**: `Source` trait allows different source types (Textbook, Report) with shared behavior
3. **JSON-Based Storage**: Simple, human-readable persistence using `projects.json`
4. **Idempotent Operations**: Commands like `chunk` can be safely run multiple times
5. **Cross-Platform**: Handles platform differences (Unix symlinks vs Windows copies) transparently

## Project Structure

```
/home/runner/work/Arcane/Arcane/
├── Cargo.toml              # Project manifest with dependencies
├── Cargo.lock              # Locked dependency versions
├── LICENSE                 # MIT License
├── README.md               # Project overview and quick start
├── USER_GUIDE.md           # End-user documentation
├── DEVELOPER_GUIDE.md      # This file
└── src/
    ├── main.rs             # CLI entry point and command handlers
    ├── models.rs           # Domain models and business logic
    ├── pdf_engine.rs       # PDF processing and chunking
    └── storage.rs          # File system and persistence layer
```

### Dependencies

**Production Dependencies:**
- `lopdf` (0.39.0) - PDF manipulation without quality loss
- `serde` (1.0) + `serde_json` (1.0) - Serialization for `projects.json`
- `anyhow` (1.0) - Ergonomic error handling

**Development Dependencies:**
- `tempfile` (3) - Temporary directories for testing

## Core Components

### 1. CLI Layer (`main.rs`)

The CLI layer provides the user interface through a hand-rolled argument parser. This keeps the binary small and avoids heavyweight CLI framework dependencies.

**Key Functions:**
- `main()`: Entry point that dispatches to command handlers
- `cmd_new()`: Creates a new project
- `cmd_list()`: Lists all projects and sources
- `cmd_show()`: Shows detailed project information
- `cmd_add()`: Adds a PDF source to a project
- `cmd_chunk()`: Splits textbooks into chapters
- `print_usage()`: Displays help message

**Example Command Flow:**
```
arcane add "Project" book.pdf --textbook --start-page 12
     ↓
cmd_add() parses arguments
     ↓
Creates/updates SourceMeta
     ↓
Stores in ProjectStore
     ↓
Saves to projects.json
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

**`SourceMeta`** (src/models.rs:54)
```rust
pub struct SourceMeta {
    pub title: String,                      // Display name
    pub path: PathBuf,                      // Path to original PDF
    pub needs_chunking: bool,               // Textbook vs Report
    pub chapter_map: HashMap<u32, String>,  // Physical page → chapter name
    pub start_page_physical: Option<u32>,   // Offset for page numbering
}
```

**`Source` trait** (src/models.rs:113)
```rust
pub trait Source {
    fn chunk(&self, chunks_dir: &Path) -> Result<()>;
    fn youtube(&self, url: &str) -> Result<()>; // Future feature
}
```

#### Trait Implementations

- **`Textbook`** (src/models.rs:134): Large PDFs that need chapter splitting
- **`Report`** (src/models.rs:156): Small PDFs stored as-is without chunking

The `build_source()` factory function (src/models.rs:181) dispatches to the correct implementation based on `needs_chunking`.

### 3. PDF Engine (`pdf_engine.rs`)

The PDF engine handles intelligent chapter detection and PDF splitting.

#### Chapter Detection Strategy (the "Xodo Method")

The engine tries three methods in order of preference:

1. **`/Outlines` (Bookmarks/TOC)** - Most reliable (src/pdf_engine.rs:143)
   - Walks the PDF outline tree
   - Extracts physical page indices from destinations
   - Uses bookmark titles as chapter names

2. **`/PageLabels` Dictionary** - Fallback (src/pdf_engine.rs:234)
   - Parses the PageLabels number tree
   - Uses range starts as chapter boundaries
   - Generates titles if not provided

3. **User-supplied `start_page_physical`** - Last resort
   - User specifies where printed "Page 1" begins
   - Helps with textbooks that have Roman numeral front matter

#### Physical vs Logical Page Mapping

Many textbooks have front matter with Roman numerals (i, ii, iii, ...) before the actual content begins at "Page 1". The PDF engine handles this through an offset calculation:

```
offset = physical_index - logical_page_number
```

For example, if printed "Page 1" appears at physical index 12:
- offset = 12 - 1 = 11
- To find physical page for logical page 25: physical = 25 + 11 = 36

#### Key Functions

- **`chunk_pdf()`** (src/pdf_engine.rs:51): Main entry point for chunking
- **`extract_chapters_from_outlines()`** (src/pdf_engine.rs:143): Parse PDF bookmarks
- **`extract_chapters_from_page_labels()`** (src/pdf_engine.rs:234): Parse page labels
- **`boundaries_to_ranges()`** (src/pdf_engine.rs:303): Convert boundaries to page ranges
- **`write_chunk()`** (src/pdf_engine.rs:330): Extract and write chapter PDFs
- **`sanitise_filename()`** (src/pdf_engine.rs:383): Clean chapter titles for filenames

#### Idempotency

The `is_already_chunked()` function (src/pdf_engine.rs:118) checks if the chunks directory already contains PDF files. If so, `chunk_pdf()` returns early without reprocessing. This makes the `chunk` command safe to run multiple times.

### 4. Storage Layer (`storage.rs`)

The storage layer manages file system structure and JSON persistence.

#### File System Layout

```
~/Arcane/
├── projects.json                    # All project metadata
└── Library/
    └── [Project_Name]/
        ├── Originals/               # Symlinks/copies to original PDFs
        └── Chunks/                  # Split chapter PDFs
```

#### Key Functions

- **`arcane_root()`** (src/storage.rs:29): Returns `~/Arcane/`
- **`project_dir()`** (src/storage.rs:38): Returns project directory
- **`originals_dir()`** (src/storage.rs:44): Returns Originals directory
- **`chunks_dir()`** (src/storage.rs:52): Returns Chunks directory
- **`link_original()`** (src/storage.rs:166): Creates symlink or copy to original PDF

#### ProjectStore

The `ProjectStore` struct (src/storage.rs:86) provides CRUD operations for projects:

```rust
pub struct ProjectStore {
    path: PathBuf,       // Path to projects.json
    data: StoreData,     // In-memory project list
}
```

**Methods:**
- `load()`: Load from `~/Arcane/projects.json`
- `save()`: Persist changes to disk
- `get()` / `get_mut()`: Query by project name
- `upsert()`: Insert or update a project
- `remove()`: Delete a project

#### Platform Differences

On Unix systems, `link_original()` creates symlinks (src/storage.rs:178). On Windows, it falls back to copying files (src/storage.rs:189) if symlinks aren't available.

## Design Philosophy

### 1. Zero Bloat

Arcane minimizes dependencies to keep the binary small and compilation fast. The CLI parser is hand-rolled rather than using a framework like `clap`.

### 2. Local-First

All data is stored locally in `~/Arcane/`. No cloud services, no network dependencies, no telemetry. Users have complete control over their data.

### 3. Idempotent Operations

Commands are designed to be safely re-runnable:
- `chunk` won't reprocess existing chapters
- `add` won't duplicate sources (though it will add again if you really want)
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

Tests are organized as inline modules at the end of each source file:

- `models.rs`: Tests for domain models and serialization (src/models.rs:193)
- `pdf_engine.rs`: Tests for boundary calculation and sanitization (src/pdf_engine.rs:401)
- `storage.rs`: Tests for ProjectStore operations (src/storage.rs:205)

Tests use `tempfile` for creating temporary directories that are automatically cleaned up.

### Code Coverage

Current test coverage:
- **models.rs**: Project operations, serialization round-trips
- **pdf_engine.rs**: Boundary detection, filename sanitization, idempotency
- **storage.rs**: CRUD operations, persistence

Areas that could use more testing:
- PDF outline parsing (requires test PDFs)
- PageLabels extraction (requires test PDFs)
- End-to-end command flows

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

1. **YouTube Integration** (src/models.rs:122)
   - Extract transcripts from lecture videos
   - Link video timestamps to textbook sections
   - Store as markdown with video embeds

2. **Search Functionality**
   - Full-text search across all PDFs
   - Search by project, title, or tags
   - Search within chapter content

3. **Tags and Filtering**
   - Filter projects by tags
   - Auto-tag based on content
   - Tag-based organization

4. **Export Functionality**
   - Export project metadata
   - Bundle projects for sharing
   - Export to other formats (Obsidian, Zotero, etc.)

5. **GUI Interface**
   - Electron or Tauri-based GUI
   - Visual project organization
   - PDF preview in app

6. **OCR Support**
   - Extract text from scanned PDFs
   - Make scanned textbooks searchable
   - Generate text summaries

### Contributing to Roadmap

Have an idea for a new feature? Open an issue on GitHub to discuss it!

## Technical Debt and Known Limitations

### Current Limitations

1. **No Remove Command**: Currently no CLI command to remove sources from projects (requires manual editing of `projects.json`)

2. **Limited Error Recovery**: If a PDF is malformed, the entire chunking operation may fail

3. **No Progress Indication**: Large PDFs don't show progress during chunking

4. **Chapter Name Accuracy**: Depends entirely on PDF metadata quality

5. **No Merge Support**: Can't merge multiple PDFs into one source

### Areas for Improvement

1. **Better Error Messages**: More specific error messages when PDF processing fails

2. **Progress Bars**: Add progress indication for long operations

3. **Batch Operations**: Support for adding multiple PDFs at once

4. **Configuration File**: Allow users to customize default paths and behavior

5. **Source Editing**: Commands to update source metadata without manual JSON editing

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

Enable verbose logging by modifying the code to print intermediate values:
```rust
println!("[DEBUG] Chapter map: {:?}", chapters);
println!("[DEBUG] Total pages: {}", total_pages);
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
