# Arcane User Guide

Welcome to Arcane! This guide will help you get started with organizing your research materials.

## Table of Contents

- [Introduction](#introduction)
- [Installation](#installation)
- [Getting Started](#getting-started)
- [Command Reference](#command-reference)
- [Common Workflows](#common-workflows)
- [PDF Analysis Pipeline](#pdf-analysis-pipeline)
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

### Building with OCR Support (Optional)

For PDFs with broken font encoding, Arcane can fall back to image-based OCR:

```bash
cargo build --release --features ocr
```

Then download all required models and runtime libraries:

```bash
arcane init-ocr
```

This downloads PaddleOCR v5 models, ONNX Runtime, and PDFium to `~/Arcane/models/`.
Everything is auto-detected at runtime — no environment variables needed.

Options:
- `--force` — re-download even if files exist
- `--skip-runtime` — skip ONNX Runtime and PDFium (only download models)
- `--models-dir <path>` — override the download directory

#### Advanced: Manual Path Overrides

For non-English PDFs or custom model locations, set these environment variables:

| Variable | Purpose |
|----------|---------|
| `ARCANE_OCR_DET_MODEL` | Detection ONNX model |
| `ARCANE_OCR_REC_MODEL` | Recognition ONNX model (e.g. Chinese) |
| `ARCANE_OCR_DICT` | Recognition dictionary |
| `ORT_DYLIB_PATH` | ONNX Runtime shared library |

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

### `arcane probe <file> [--json]`

Classifies a PDF as text-based, scanned (image-only), or mixed. Inspects every page's content stream for text-showing operators vs image XObjects.

**Options:**
- `--json`: Output the full result as JSON (including per-page breakdown)

**Example:**
```bash
arcane probe ~/Books/textbook.pdf
```

**Output:**
```
File:         /home/user/Books/textbook.pdf
Pages:        420
Type:         Text-Based
Text pages:   420
Image pages:  0
Has outlines: no
Page labels:  yes
```

This is the first step in understanding whether a PDF needs outline recovery, OCR, or is already well-structured.

### `arcane detect-layout <file> [--json] [--pages RANGE]`

Extracts positioned text from every page using the PDF text-matrix state machine, builds a statistical typographic profile over the first 50 pages, and classifies structural anchors — chapter headings, section headings, numbered headings, TOC entries, and page numbers — using Z-score thresholds and Bayesian confidence scoring.

**How it works:**

1. **Text extraction** — reads every PDF text operator (`Tj`, `TJ`, `'`, `"`) and corrects effective font sizes using the text matrix (`Tm`). This fixes a common problem where PDFs store a nominal `1pt` font size but scale it to 14pt via the text matrix — without this correction, `body_font_size` would report as `1.0`.

2. **Typographic profiling** — computes mean (μ), standard deviation (σ), and mode (body centroid) of all effective font sizes, plus the 90th-percentile vertical gap between text blocks.

3. **Feature extraction** — assigns each text run a `TextFeature` with:
   - Z-score: `(size − μ) / σ`
   - Flags: `BOLD`, `ITALIC`, `ALL_CAPS`, `TITLE_CASE`, `ISOLATED` (gap ≥ p90), `LARGE_FONT` (z > 3.0), `MED_FONT` (z ∈ [1.5, 3.0))
   - Case pattern: AllCaps / TitleCase / SentenceCase / Mixed / Numeric

4. **Classification** — applies a priority rule table (large+bold+isolated → ChapterHeading; bold+isolated+body-size → SectionHeading; etc.) then boosts confidence +0.20 for TOC fuzzy matches ≥ 0.80. Anchors below 0.40 confidence are dropped.

**Options:**
- `--json`: Output the full result as JSON (anchors, font clusters, feature vectors)
- `--pages RANGE`: Only analyse specific pages (0-based range, e.g. "0-5")

**Example:**
```bash
arcane detect-layout ~/Books/textbook.pdf --json
```

**Output (human-readable):**
```
File:           /home/user/Books/textbook.pdf
Pages:          420
Body font size: 10.0pt

Font clusters:
  24.0pt       150 chars  Heading1
  14.0pt       800 chars  Heading2
  10.0pt    250000 chars  Body
   8.0pt      5000 chars  Footnote

Structural anchors (42):
  page    3  y= 650.0  ChapterHeading     24.0  Introduction
  page    3  y= 580.0  SectionHeading     14.0  1.1 Background
  ...
```

> **Note for LaTeX/scanned books:** Some PDFs (especially those built with LaTeX or tools that strip metadata) store nominal `1pt` font sizes and scale via the text matrix. Before the Tm-scale fix, `detect-layout` would report `body_font_size: 1.0` and fail to detect any headings. The fix resolves this automatically.

### `arcane find-offset <file> [--toc-pages <range>] [--json]`

Calculates the integer delta between printed page numbers and physical PDF page indices. For example, if the printed "page 1" starts at physical page 19, the offset is +18.

Uses three strategies in priority order:
1. `/PageLabels` number tree (fastest, most reliable when present)
2. TOC matching — fuzzy-matches TOC-entry titles against page text at candidate offsets
3. Page-number detection — finds printed numbers in headers/footers and uses consensus

**Options:**
- `--toc-pages START-END`: TOC page range (1-based, e.g. "3-5") for targeted matching
- `--json`: Output as JSON

**Example:**
```bash
arcane find-offset ~/Books/textbook.pdf
arcane find-offset ~/Books/textbook.pdf --toc-pages 7-12
```

**Output:**
```
File:       /home/user/Books/textbook.pdf
Offset:     +18
Confidence: 95%
Method:     PageLabels

Evidence:
  physical page   18 → printed page    1  (PageLabels: Arabic numbering starts at physical page 18)
```

### `arcane sync-pages <file> [--toc-pages RANGE] [--threshold T] [--json]`

Correlates detected chapter headings with TOC entries using a RANSAC-style consensus algorithm to determine the physical-to-logical page offset. More robust than `find-offset` when the PDF has no `/PageLabels` and the TOC is noisy.

**How it works:**

1. Runs the full layout analysis to detect chapter and section headings.
2. Parses TOC entries (title + printed page number) from the specified (or auto-detected) TOC pages.
3. For every heading × TOC-entry pair whose title similarity ≥ `--threshold`, computes a candidate delta: `physical_page − printed_page`.
4. Builds a histogram of all candidate deltas weighted by similarity score.
5. The **consensus offset** is the delta with the highest total weight. Inliers are all pairs where `|delta − consensus| ≤ 1`.
6. Re-runs classification with Bayesian confidence boosts for the consensus offset.

**Options:**
- `--toc-pages START-END`: TOC page range (1-based, e.g. "14-20"). Auto-detected if omitted.
- `--threshold T`: Minimum normalised Levenshtein similarity for a heading↔TOC match (default: 0.6)
- `--json`: Output as JSON (consensus offset, confidence, inlier match table)

**Example:**
```bash
arcane sync-pages ~/Books/textbook.pdf --toc-pages 14-20
```

**Output:**
```
File:             /home/user/Books/textbook.pdf
Consensus offset: +18
Confidence:       92% (11/12 inliers)

Matches:
  TOC: "Introduction"  printed p.1   →  physical p.19  similarity=0.97  ✓
  TOC: "Foundations"   printed p.21  →  physical p.39  similarity=0.92  ✓
  TOC: "Sorting"       printed p.43  →  physical p.61  similarity=0.89  ✓
  ...
```

**When to use `sync-pages` vs `find-offset`:**
- `find-offset` is faster and works well when `/PageLabels` exists or the TOC is small.
- `sync-pages` is the right choice for books with no `/PageLabels`, large/complex TOCs, or when you need a match confidence table for verification.

### `arcane recover-outline <file> [options]`

Recovers and injects outline bookmarks into PDFs that have no `/Outlines`, using a tiered pipeline:

1. **Probe** — classify the document (text-based or scanned)
2. **Profile** — build a statistical typographic profile (μ, σ, body centroid, gap p90) over the first 50 pages
3. **Classify** — extract `TextFeature` vectors (Z-score, bold/italic flags, case pattern, isolation) and classify structural anchors with Bayesian confidence scoring
4. **Offset** — calculate the front-matter page delta
5. **Verify** — fuzzy-match each heading against the text on its target page
6. **Inject** — write a hierarchical `/Outlines` tree (Chapter > Section nesting)

**Options:**
- `--output PATH`: Write the fixed PDF to a new file instead of overwriting the input
- `--dry-run`: Preview detected headings without writing anything
- `--min-font-ratio R`: Font-size multiplier above body text to classify as a heading (default: 1.2)
- `--depth N`: Maximum heading depth to inject (1 = chapter-level only, 2 = chapters + sections; default: 2)
- `--toc-pages START-END`: TOC page range (1-based, e.g. "7-12") for deterministic matching — bypasses TOC discovery and speeds up the process by ~40%
- `--no-inject`: Run the full pipeline but skip injection (useful with `--json` for inspection)
- `--fuzzy-threshold T`: Minimum similarity (0.0–1.0) for heading verification (default: 0.6)
- `--json`: Output the full pipeline result as JSON (probe, layout, offset, headings, verification)

**Workflow:**
```bash
# 1. Classify the PDF
arcane probe "book.pdf"

# 2. Preview detected headings and verification results
arcane recover-outline "book.pdf" --dry-run

# 3. If too many headings, raise the ratio; if too few, lower it
arcane recover-outline "book.pdf" --dry-run --min-font-ratio 1.4

# 4. For deterministic results, specify the TOC pages
arcane recover-outline "book.pdf" --dry-run --toc-pages 7-12

# 5. Generate a fixed copy with injected bookmarks
arcane recover-outline "book.pdf" --output "book-recovered.pdf"

# 6. Get full pipeline output as JSON for inspection
arcane recover-outline "book.pdf" --no-inject --json

# 7. Verify the injected outlines look correct
arcane outline "book-recovered.pdf" --depth 3

# 8. Replace the source in your project and re-chunk
arcane remove "Project" "Book Title"
arcane add    "Project" "book-recovered.pdf" --textbook --title "Book Title"
arcane chunk  "Project" --source "Book Title" --depth 1
```

**Tips:**
- Use `arcane probe` first to confirm the PDF is text-based (scanned PDFs require the `--features ocr` build)
- Increase `--min-font-ratio` (e.g. to 1.4) if too many section headings are detected
- Use `--depth 1` for chapter-level chunks only; `--depth 2` splits at section level too
- The `--toc-pages` flag provides ~40% speedup and 100% deterministic matching when you know where the TOC is
- LaTeX books (Computer Modern fonts: CMBX12, CMBX17) work particularly well
- Use `--json` to pipe results into other tools or scripts

### `arcane init-ocr [--models-dir DIR] [--skip-runtime] [--force]`

Downloads all OCR model files and runtime libraries (ONNX Runtime, PDFium) for the current platform to `~/Arcane/models/`. Files are auto-detected at runtime — no environment variables needed.

**Options:**
- `--models-dir DIR`: Override the download directory (default: `~/Arcane/models/`)
- `--skip-runtime`: Only download model files, skip ONNX Runtime and PDFium DLLs
- `--force`: Re-download files even if they already exist

**Example:**
```bash
# Download everything (first-time setup)
arcane init-ocr

# Force re-download
arcane init-ocr --force

# Models only (you already have ONNX Runtime and PDFium)
arcane init-ocr --skip-runtime
```

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

## PDF Analysis Pipeline

Arcane includes a full pipeline for analysing and recovering the structure of PDFs that lack bookmarks. The commands build on each other:

```
arcane probe book.pdf                  # Step 1: Is it text-based or scanned?
arcane detect-layout book.pdf          # Step 2: What headings / font distribution does it have?
arcane sync-pages book.pdf             # Step 3: Find the physical↔printed page offset (RANSAC)
arcane find-offset book.pdf            # Step 3 alt: Simpler offset detection (PageLabels / TOC)
arcane recover-outline book.pdf        # Step 4: Inject recovered bookmarks into the PDF
arcane outline book-recovered.pdf      # Step 5: Verify the injected outline looks right
```

### Workflow: Fixing a Book with No Bookmarks

```bash
# Confirm the PDF is text-based (not scanned)
arcane probe ~/Books/vision.pdf

# Preview the structural analysis — check body_font_size is sensible (e.g. ~10pt)
arcane detect-layout ~/Books/vision.pdf

# If the book has a TOC, use sync-pages to find the page offset
arcane sync-pages ~/Books/vision.pdf --toc-pages 14-20

# Preview detected headings with the recovered structure
arcane recover-outline ~/Books/vision.pdf --dry-run --toc-pages 14-20

# Write a fixed copy with injected bookmarks
arcane recover-outline ~/Books/vision.pdf --output ~/Books/vision-fixed.pdf --toc-pages 14-20

# Verify bookmarks were injected correctly
arcane outline ~/Books/vision-fixed.pdf --depth 3

# Add the fixed copy to your project and chunk it
arcane add "Computer Vision" ~/Books/vision-fixed.pdf --textbook --title "3-D Vision"
arcane chunk "Computer Vision" --source "3-D Vision" --depth 1
```

### Understanding `body_font_size`

The `body_font_size` field in `detect-layout` output is the mode of the effective font-size distribution — the size that the majority of body text uses. It is derived from the full typographic profile (μ, σ, histogram mode over 50 pages), so it is robust against outliers like large chapter titles or tiny footnotes.

> **Before the Tm-scale fix:** PDFs built with tools that store a nominal `1pt` font size scaled by the text matrix would report `body_font_size: 1.0` and detect zero headings. The fix tracks the `Tm` operator to compute `effective_size = nominal × √(a²+b²)`, resolving the issue.

### Confidence Scores

Every structural anchor detected by `detect-layout` and `recover-outline` has a confidence score between 0.0 and 1.0:

| Score range | Interpretation |
|-------------|----------------|
| 0.85 – 1.0  | Very high — large font + bold + isolated + TOC match |
| 0.65 – 0.85 | High — bold or case-pattern match with isolation |
| 0.40 – 0.65 | Medium — single signal (font size alone) |
| < 0.40      | Dropped — insufficient evidence |

The Bayesian boosts applied:
- **+0.20** when the anchor text fuzzy-matches a TOC entry title (≥ 0.80 Levenshtein similarity)
- **+0.10** when the anchor's physical page equals the expected page from the consensus offset

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

1. **The PDF doesn't have chapter metadata**: Some PDFs don't have bookmarks or page labels. Use `arcane outline <file>` to check. If there are no outlines, use `arcane recover-outline` to reconstruct them from font-size analysis.

2. **The source wasn't marked as a textbook**: Make sure you used the `--textbook` flag when adding the source.

3. **The chunks already exist**: Arcane won't re-process existing chunks. Use `--force` to regenerate.

4. **Outline depth too shallow**: The PDF may have chapters at a deeper outline level. Try `--depth 2` or `--depth 3`.

5. **The PDF is a scanned image**: Use `arcane probe <file>` to check. If the PDF is scanned (image-only), build with `--features ocr` to enable OCR-based text recognition (see [Building with OCR Support](#building-with-ocr-support-optional) above).

**Solution:**
```bash
# Classify the PDF first
arcane probe ~/path/to/book.pdf

# Inspect the PDF's outline
arcane outline ~/path/to/book.pdf --depth 3

# If no outlines exist, recover them
arcane recover-outline ~/path/to/book.pdf --dry-run
arcane recover-outline ~/path/to/book.pdf --output ~/path/to/book-fixed.pdf

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
