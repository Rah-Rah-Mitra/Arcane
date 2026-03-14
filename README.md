# Arcane

A local-first research archival application for organizing academic materials. Arcane helps researchers and students manage PDF documents like textbooks, research papers, and lecture notes.

## Features

- **Project-Based Organization**: Group related sources under projects with optional tags
- **Smart PDF Processing**: Automatically split textbooks into individual chapter files
- **Intelligent Chapter Detection**: Extracts chapters from PDF metadata (bookmarks/outlines or page labels)
- **Physical/Logical Page Mapping**: Handles textbooks with Roman numeral front matter
- **Local-First Architecture**: All data stored locally with no cloud dependency
- **Cross-Platform**: Works on Unix (with symlinks) and Windows (with file copies)
- **Zero Bloat**: Minimal dependencies with a hand-rolled CLI parser

## Quick Start

### Installation

```bash
# Build from source
git clone https://github.com/Rah-Rah-Mitra/Arcane.git
cd Arcane
cargo build --release

# Install locally
cargo install --path .
```

### Basic Usage

```bash
# Create a new project
arcane new "Algorithms"

# Add a research paper (small PDF that doesn't need splitting)
arcane add "Algorithms" ~/Documents/quicksort-paper.pdf

# Add a textbook (will be split into chapters)
arcane add "Algorithms" ~/Documents/clrs.pdf --textbook --start-page 12

# Split textbooks into chapters
arcane chunk "Algorithms"

# List all projects and their sources
arcane list

# Show project details
arcane show "Algorithms"
```

## How It Works

Arcane organizes your research materials in a structured filesystem:

```
~/Arcane/
├── projects.json                    # Project metadata
└── Library/
    └── [Project_Name]/
        ├── Originals/               # Links to original PDFs
        └── Chunks/                  # Split chapter PDFs
```

When you add a textbook source, Arcane:
1. Creates a symlink (or copy on Windows) to the original PDF
2. Stores metadata in `projects.json`
3. When you run `chunk`, extracts chapter boundaries from the PDF
4. Splits the textbook into individual chapter files

## Documentation

- [User Guide](USER_GUIDE.md) - Detailed usage instructions and workflows
- [Developer Guide](DEVELOPER_GUIDE.md) - Architecture overview and contributing guidelines

## Requirements

- Rust 1.93.1 or later
- Cargo

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please see the [Developer Guide](DEVELOPER_GUIDE.md) for information on the codebase architecture and how to contribute.
