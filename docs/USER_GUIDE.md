# Arcane User Guide

Welcome to Arcane! This guide will help you get started with organizing your research materials.

## Table of Contents

- [Introduction](#introduction)
- [Installation](#installation)
- [Getting Started](#getting-started)
- [Command Reference](#command-reference)
- [Common Workflows](#common-workflows)
- [Understanding the Filesystem](#understanding-the-filesystem)
- [Removing Sources and Projects](#removing-sources-and-projects)
- [Troubleshooting](#troubleshooting)

## Introduction

Arcane is a local-first research archival application designed for researchers, students, and academics who need to manage PDF documents like textbooks, research papers, and lecture notes. Unlike cloud-based solutions, all your data stays on your machine, giving you full control over your academic materials.

### Key Concepts

- **Project**: A collection of related sources (e.g., "Machine Learning", "Algorithms", "Physics")
- **Source**: A PDF document that belongs to a project
- **Textbook**: A large PDF that needs to be split into individual chapters
- **Report**: A smaller PDF (like a research paper) that doesn't need splitting
- **Chunking**: The process of splitting a textbook into individual chapter PDFs

## Installation

### Prerequisites

- Rust 1.93.1 or later
- Cargo (comes with Rust)

### Building from Source

```bash
# Clone the repository
git clone https://github.com/Rah-Rah-Mitra/Arcane.git
cd Arcane

# Build the release version
cargo build --release

# The binary will be at target/release/arcane
```

### Installing Locally

To install Arcane so you can use it from anywhere:

```bash
cargo install --path .
```

This will install the `arcane` command in your Cargo bin directory (usually `~/.cargo/bin/`), which should be in your PATH.

### Verifying Installation

```bash
arcane --help
```

You should see the help message with all available commands.

## Getting Started

### Creating Your First Project

Let's create a project for organizing algorithms materials:

```bash
arcane new "Algorithms"
```

This creates:
- An entry in `~/Arcane/projects.json`
- Directory structure: `~/Arcane/Library/Algorithms/Originals/` and `~/Arcane/Library/Algorithms/Chunks/`

### Adding a Research Paper

Research papers are typically small PDFs that don't need to be split:

```bash
arcane add "Algorithms" ~/Documents/quicksort-paper.pdf
```

### Adding a Textbook

Textbooks are large PDFs that you want to split into chapters:

```bash
arcane add "Algorithms" ~/Documents/clrs.pdf --textbook
```

If your textbook has front matter (preface, table of contents) with Roman numerals, and the actual content starts at physical page 12, you can specify that:

```bash
arcane add "Algorithms" ~/Documents/clrs.pdf --textbook --start-page 12
```

This helps Arcane correctly map the printed page numbers to the PDF pages.

### Splitting Textbooks into Chapters

After adding textbooks, split them into chapters:

```bash
arcane chunk "Algorithms"
```

Arcane will:
1. Read the PDF's outline/bookmarks or page labels
2. Extract chapter boundaries
3. Create individual PDF files for each chapter in `~/Arcane/Library/Algorithms/Chunks/<Source Title>/`

The files will be named like: `d1_01_Introduction.pdf`, `d1_02_Getting_Started.pdf`, etc. The `d1` prefix indicates the outline depth used (depth 1 = top-level chapters). Each source gets its own subdirectory under `Chunks/`.

### Viewing Your Projects

List all projects and their sources:

```bash
arcane list
```

Show detailed information about a specific project:

```bash
arcane show "Algorithms"
```

## Command Reference

### `arcane new <project>`

Creates a new project.

**Example:**
```bash
arcane new "Machine Learning"
```

### `arcane list`

Lists all projects and their sources.

**Example:**
```bash
arcane list
```

**Output:**
```
  • Algorithms
      Introduction to Algorithms (textbook) — /home/user/Documents/clrs.pdf
      Quicksort Analysis (report) — /home/user/Documents/quicksort.pdf
```

### `arcane show <project>`

Shows detailed information about a project, including source metadata such as chunking depth, page count, and number of chunks generated.

**Example:**
```bash
arcane show "Algorithms"
```

**Output:**
```
Project : Algorithms
Sources :
  • Introduction to Algorithms — /home/user/Documents/clrs.pdf
    needs_chunking = true
    start_page_physical = 12
    chapter_map = 34 entries
    depth = 2
    page_count = 1312
    chunks = 34
```

### `arcane add <project> <pdf-path> [options]`

Adds a PDF source to a project.

**Options:**
- `--textbook`: Mark the source as a textbook that needs chunking
- `--start-page N`: Physical page index where printed Page 1 starts (for textbooks with front matter)
- `--title T`: Override the display title (defaults to filename)
- `--tag TAG`: Add a tag to the project (can be repeated, e.g. `--tag math --tag algorithms`)
- `--type TYPE`: Source type label: textbook, report, paper, cheatsheet, or any custom string

**Examples:**

Add a research paper:
```bash
arcane add "Algorithms" ~/Downloads/paper.pdf
```

Add a textbook:
```bash
arcane add "Algorithms" ~/Books/algorithms.pdf --textbook
```

Add a textbook with front matter:
```bash
arcane add "Algorithms" ~/Books/book.pdf --textbook --start-page 15
```

Add with a custom title and tags:
```bash
arcane add "Algorithms" ~/Books/book.pdf --textbook --title "Algorithms Textbook 2023" --tag math
```

Add with a custom source type:
```bash
arcane add "Algorithms" ~/Papers/survey.pdf --type paper
```

**Note:** If the project doesn't exist, it will be created automatically.

### `arcane chunk <project> [--force] [--depth N] [--dry-run] [--source S]`

Splits textbook sources in a project into individual chapter PDFs.

**Options:**
- `--force`: Delete existing chunks and regenerate them
- `--depth N`: How many levels of the outline tree to use (default: 1 = top-level chapters only; 2 = chapters + sections; etc.)
- `--dry-run`: Preview detected chapter boundaries without writing any files
- `--source S`: Only chunk a specific source (by title). This allows different textbooks to be chunked at different depths, since each textbook may have a different level of hierarchy in its table of contents

**Examples:**

Basic chunking (all textbook sources):
```bash
arcane chunk "Algorithms"
```

Preview what chapters will be detected:
```bash
arcane chunk "Algorithms" --dry-run
```

Force re-chunking with sub-chapter granularity:
```bash
arcane chunk "Algorithms" --force --depth 2
```

Chunk a specific textbook at its own depth:
```bash
arcane chunk "Algorithms" --source "CLRS 4th Edition" --depth 2
arcane chunk "Algorithms" --source "Sedgewick & Wayne" --depth 1
```

Chunk files are named with a depth prefix and chapter title, e.g. `d2_01_Introduction.pdf`, `d2_02_Sorting.pdf`. This distinguishes chunks created at different depths.

This command is idempotent—running it multiple times won't re-process chapters that already exist (unless `--force` is used).

### `arcane list-chunks <project> [source]`

Lists the chunk files for a source in a project.

**Examples:**

List chunks for all sources:
```bash
arcane list-chunks "Algorithms"
```

List chunks for a specific source:
```bash
arcane list-chunks "Algorithms" "CLRS 4th Edition"
```

### `arcane search <query> [--limit N] [--project P] [--source S]`

Searches across all indexed sources. Optionally filter by project or source.

**Options:**
- `--limit N`: Maximum number of results (default: 10)
- `--project P`: Only search within a specific project
- `--source S`: Only search within a specific source title

**Examples:**

Search everywhere:
```bash
arcane search "dynamic programming"
```

Search within one project:
```bash
arcane search "quicksort" --project "Algorithms"
```

Search within one source:
```bash
arcane search "binary heap" --source "CLRS 4th Edition"
```

### `arcane outline <file> [--depth N]`

Displays the outline (bookmarks) and page labels of any PDF file. Useful for inspecting a PDF before adding it to a project.

**Options:**
- `--depth N`: Maximum depth of outline entries to display (default: 10)

**Example:**
```bash
arcane outline ~/Books/algorithms.pdf --depth 3
```

**Output:**
```
File: /home/user/Books/algorithms.pdf
Total pages: 1312

Outlines (depth 3):
  #    Title                                              Pages
  ──────────────────────────────────────────────────────────────────────
  01   Front Matter                                       1-18
  02   I Foundations                                      19-20
  03   I Foundations > 1 The Role of Algorithms           21-36
  ...

Page labels:
  Page 1 — Front Matter (roman)
  Page 19 — Content (arabic)
```

### `arcane recover-outline <file> [options]`

Re-hydrates the outline (bookmarks) of a PDF that has no `/Outlines` and no `/PageLabels` by analysing font sizes in the content streams. Useful for LaTeX-generated textbooks whose metadata was stripped.

**Options:**
- `--output PATH`: Write the fixed PDF to a new file instead of overwriting the input
- `--dry-run`: Preview detected headings without writing anything
- `--min-font-ratio R`: Font-size multiplier above body text to classify as a heading (default: 1.2 = 20 % larger)
- `--depth N`: Maximum heading depth to inject (1 = chapter-level only, 2 = chapters + sections; default: 2)

**Workflow:**
```bash
# 1. Confirm the PDF has no outlines
arcane outline "book.pdf"

# 2. Preview detected headings (adjust --min-font-ratio if needed)
arcane recover-outline "book.pdf" --dry-run

# 3. Generate a fixed copy with injected bookmarks
arcane recover-outline "book.pdf" --output "book-recovered.pdf"

# 4. Verify the injected outlines look correct
arcane outline "book-recovered.pdf" --depth 3

# 5. Replace the source in your project and re-chunk
arcane remove "Project" "Book Title"
arcane add    "Project" "book-recovered.pdf" --textbook --title "Book Title"
arcane chunk  "Project" --source "Book Title" --depth 1
```

**Tips:**
- Increase `--min-font-ratio` (e.g. to 1.4) if too many section headings are detected
- Use `--depth 1` for chapter-level chunks only; `--depth 2` splits at section level too
- LaTeX books (Computer Modern fonts: CMBX12, CMBX17) work particularly well

### `arcane remove <project> [source]`

Removes a source from a project, or an entire project if no source is specified. Cleans up the database, search index, and filesystem.

**Examples:**

Remove a single source:
```bash
arcane remove "Algorithms" "CLRS 4th Edition"
```

Remove an entire project and all its sources:
```bash
arcane remove "Algorithms"
```

### `arcane --help` or `arcane -h`

Shows the help message with all available commands.

## Common Workflows

### Workflow 1: Organizing a Course

Let's say you're taking a Machine Learning course and want to organize all materials:

```bash
# Create the project
arcane new "Machine Learning Spring 2026"

# Inspect the textbook's outline before adding
arcane outline ~/Books/ml-textbook.pdf --depth 2

# Add the main textbook
arcane add "Machine Learning Spring 2026" ~/Books/ml-textbook.pdf --textbook --start-page 10

# Add lecture notes
arcane add "Machine Learning Spring 2026" ~/Lectures/week1.pdf

# Add research papers
arcane add "Machine Learning Spring 2026" ~/Papers/neural-networks.pdf
arcane add "Machine Learning Spring 2026" ~/Papers/deep-learning.pdf

# Preview chunk boundaries
arcane chunk "Machine Learning Spring 2026" --dry-run --depth 2

# Split the textbook into chapters (with sub-chapter depth)
arcane chunk "Machine Learning Spring 2026" --depth 2

# View everything
arcane show "Machine Learning Spring 2026"
arcane list-chunks "Machine Learning Spring 2026"

# Search within this project only
arcane search "gradient descent" --project "Machine Learning Spring 2026"
```

### Workflow 2: Research Paper Collection

If you're collecting papers on a specific topic:

```bash
# Create a project
arcane new "Neural Networks Research"

# Add multiple papers
arcane add "Neural Networks Research" ~/Papers/backprop.pdf
arcane add "Neural Networks Research" ~/Papers/cnns.pdf
arcane add "Neural Networks Research" ~/Papers/rnns.pdf

# List all papers
arcane list
```

### Workflow 3: Multiple Textbooks for One Topic

When studying a topic using multiple textbooks:

```bash
# Create the project
arcane new "Algorithms Study"

# Add textbooks with custom titles
arcane add "Algorithms Study" ~/Books/clrs.pdf --textbook --title "CLRS 4th Edition" --start-page 12
arcane add "Algorithms Study" ~/Books/dasgupta.pdf --textbook --title "Algorithms by Dasgupta"
arcane add "Algorithms Study" ~/Books/sedgewick.pdf --textbook --title "Sedgewick & Wayne"

# Chunk all textbooks
arcane chunk "Algorithms Study"

# Now you have chapters from all three books organized
arcane show "Algorithms Study"
arcane list-chunks "Algorithms Study"

# Search a specific book
arcane search "red-black trees" --source "CLRS 4th Edition"

# Later, remove a source you no longer need
arcane remove "Algorithms Study" "Sedgewick & Wayne"
```

## Understanding the Filesystem

Arcane stores everything in `~/Arcane/`:

```
~/Arcane/
├── projects.json                                # All project metadata
├── arcane.db                                    # SQLite database
├── CAS/                                         # Content-addressed blob store
└── Library/
    ├── Algorithms/
    │   ├── Originals/
    │   │   ├── clrs.pdf                        # Symlink (Unix) or copy (Windows)
    │   │   └── quicksort-paper.pdf
    │   └── Chunks/
    │       └── Introduction to Algorithms/     # Per-source subdirectory
    │           ├── d1_01_Front_Matter.pdf       # Depth-prefixed chunk files
    │           ├── d1_02_The_Role_of_Algorithms.pdf
    │           └── ...
    └── Machine Learning/
        ├── Originals/
        │   └── ml-textbook.pdf
        └── Chunks/
            └── ML Textbook/
                └── ...
```

### projects.json

This file contains all project metadata, including:
- Project names and tags
- List of sources in each project
- Source metadata (title, path, type, chapter information)

**Note:** You can safely backup this file along with the `Library/` directory to preserve your entire archive.

### Originals Directory

Contains symlinks (or copies on Windows) to the original PDF files. This allows you to keep your PDFs in their original location while Arcane organizes them.

### Chunks Directory

Contains the individual chapter PDFs extracted from textbooks. Each source gets its own subdirectory (e.g., `Chunks/CLRS 4th Edition/`). Files are named with a depth prefix, a sequence number, and the chapter title (e.g., `d1_01_Introduction.pdf`, `d2_03_Sorting > Quicksort.pdf`). The depth prefix (`d1`, `d2`, etc.) indicates the outline depth used during chunking.

## Removing Sources and Projects

### Removing a Source

To remove a single source from a project:

```bash
arcane remove "Algorithms" "CLRS 4th Edition"
```

This will:
- Remove the source from the database and project metadata
- Delete the symlink/copy in `Originals/`
- Delete the chunk directory for that source
- Remove the source from the search index

### Removing a Project

To remove an entire project and all its sources:

```bash
arcane remove "Algorithms"
```

This will:
- Remove the project and all sources from the database
- Delete the entire `~/Arcane/Library/Algorithms/` directory
- Remove all sources from the search index

**Note:** CAS blobs are not deleted when removing sources or projects. They remain in the content-addressed store for potential reuse.

## Troubleshooting

### My textbook wasn't split into chapters

**Possible causes:**

1. **The PDF doesn't have chapter metadata**: Some PDFs don't have bookmarks or page labels. Use `arcane outline <file>` to check. Try using the `--start-page` option to help Arcane map pages correctly.

2. **The source wasn't marked as a textbook**: Make sure you used the `--textbook` flag when adding the source.

3. **The chunks already exist**: Arcane won't re-process existing chunks. Use `--force` to regenerate.

4. **Outline depth too shallow**: The PDF may have chapters at a deeper outline level. Try `--depth 2` or `--depth 3`.

**Solution:**
```bash
# Inspect the PDF's outline first
arcane outline ~/path/to/book.pdf --depth 3

# Preview what chapters will be detected
arcane chunk "Project" --dry-run --depth 2

# Force re-chunk with deeper outline parsing
arcane chunk "Project" --force --depth 2

# Or remove and re-add with correct flags
arcane remove "Project" "book"
arcane add "Project" ~/path/to/book.pdf --textbook --start-page 1
arcane chunk "Project"
```

### Chapter names are incorrect

Arcane extracts chapter names from the PDF's metadata (bookmarks/outline). If the original PDF has incorrect or missing chapter names, the extracted chapters will have those same names.

You can manually rename the chapter files in `~/Arcane/Library/[Project]/Chunks/` if needed.

### Original PDF moved or deleted

If you move or delete an original PDF, the symlink in `Originals/` will break. You'll need to:
1. Update the path in `projects.json` manually, or
2. Remove and re-add the source

### Permission errors on Linux/Mac

If you get permission errors when creating symlinks:
```bash
# Make sure ~/Arcane/ directory has correct permissions
chmod -R u+w ~/Arcane/
```

### Windows: Symlinks not working

On Windows, Arcane will automatically fall back to copying files if symlink creation fails. This means your PDFs will be duplicated rather than linked.

## Tips and Best Practices

1. **Organize by topic or course**: Create separate projects for different courses or research topics

2. **Use descriptive project names**: "Machine Learning Spring 2026" is better than "ML"

3. **Specify start-page for textbooks**: If your textbook has front matter with Roman numerals, use `--start-page` to help Arcane map pages correctly

4. **Use custom titles**: Override the filename with `--title` for better organization

5. **Backup regularly**: Backup both `~/Arcane/projects.json` and `~/Arcane/Library/` to preserve your archive

6. **Keep originals organized**: While Arcane creates symlinks, keeping your original PDFs organized helps maintain the archive

## Getting Help

- Run `arcane --help` to see all available commands
- Check the [Developer Guide](DEVELOPER_GUIDE.md) if you want to contribute or understand the internals
- Report issues at: https://github.com/Rah-Rah-Mitra/Arcane/issues
