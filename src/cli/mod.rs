//! CLI layer — clap command definitions and dispatch.

pub mod commands;
pub mod output;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Arcane — local-first research archival application.
#[derive(Parser)]
#[command(name = "arcane", about = "Local-first research archival for PDFs")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new project.
    New {
        /// Project name (used as directory name under ~/Arcane/Library/).
        name: String,
    },

    /// List all projects and their sources.
    List,

    /// Show details for a project.
    Show {
        /// Project name.
        name: String,
    },

    /// Add a source PDF to a project.
    Add {
        /// Project name (created automatically if it doesn't exist).
        project: String,

        /// Path to the PDF file.
        path: PathBuf,

        /// Mark this source as a textbook (needs chunking).
        #[arg(long)]
        textbook: bool,

        /// Physical page index where printed Page 1 starts.
        #[arg(long)]
        start_page: Option<u32>,

        /// Override the source display title.
        #[arg(long)]
        title: Option<String>,

        /// Tags to apply to the project (can be repeated).
        #[arg(long = "tag")]
        tags: Vec<String>,

        /// Source type: textbook, report, paper, cheatsheet, or custom string.
        #[arg(long = "type", value_name = "TYPE")]
        source_type: Option<String>,
    },

    /// Split textbook sources into per-chapter PDFs.
    Chunk {
        /// Project name.
        project: String,

        /// Force re-chunking even if chunks already exist.
        #[arg(long)]
        force: bool,

        /// Outline depth to use for chunking (1 = top-level only, 2+ = sub-chapters).
        #[arg(long, default_value_t = 1)]
        depth: u32,

        /// Preview detected chapter boundaries without writing files.
        #[arg(long)]
        dry_run: bool,

        /// Only chunk a specific source (by title). Allows per-textbook depth.
        #[arg(long)]
        source: Option<String>,
    },

    /// Merge multiple PDF files into one.
    Merge {
        /// Output PDF file path.
        output: PathBuf,

        /// Input PDF files to merge (in order).
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },

    /// Split a PDF into multiple files by page ranges.
    Split {
        /// Input PDF file.
        input: PathBuf,

        /// Output directory for split files.
        #[arg(long, default_value = ".")]
        output_dir: PathBuf,

        /// Page ranges (e.g., "1-5" "6-10"). 1-based, inclusive.
        #[arg(required = true)]
        ranges: Vec<String>,
    },

    /// Rotate pages in a PDF.
    Rotate {
        /// Input PDF file.
        input: PathBuf,

        /// Rotation in degrees (must be multiple of 90).
        #[arg(long, default_value_t = 90)]
        degrees: i32,

        /// Output PDF file path (defaults to overwriting input).
        #[arg(long)]
        output: Option<PathBuf>,

        /// Specific pages to rotate (0-based). If omitted, all pages are rotated.
        #[arg(long)]
        pages: Vec<u32>,
    },

    /// Add a tag to a project.
    Tag {
        /// Project name.
        project: String,

        /// Tag to add.
        tag: String,
    },

    /// Remove a tag from a project.
    Untag {
        /// Project name.
        project: String,

        /// Tag to remove.
        tag: String,
    },

    /// Search across all indexed sources.
    Search {
        /// Search query string.
        query: String,

        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,

        /// Filter results to a specific project.
        #[arg(long)]
        project: Option<String>,

        /// Filter results to a specific source title.
        #[arg(long)]
        source: Option<String>,
    },

    /// Rebuild the full-text search index from all sources.
    Reindex,

    /// Launch the interactive terminal UI.
    Tui,

    /// Watch a project directory for new PDFs.
    Watch {
        /// Project name to watch.
        project: String,
    },

    /// List chunk files for a source in a project.
    ListChunks {
        /// Project name.
        project: String,

        /// Source title. If omitted, lists chunks for all sources.
        source: Option<String>,
    },

    /// Detect layout structure and output structural anchors as JSON.
    ///
    /// Extracts positioned text from every page, clusters font sizes, and
    /// identifies headings, TOC entries, and page numbers by spatial analysis.
    DetectLayout {
        /// Path to the PDF file.
        file: PathBuf,

        /// Output as JSON (default; human-readable summary if omitted).
        #[arg(long)]
        json: bool,

        /// Only analyse specific pages (0-based range, e.g. "0-5").
        #[arg(long)]
        pages: Option<String>,
    },

    /// Calculate the logical-to-physical page offset for a PDF.
    ///
    /// Determines the integer delta between printed page numbers and PDF page
    /// indices.  For example, if the printed "page 1" starts at physical page
    /// 19 in the PDF, the offset is +18.
    FindOffset {
        /// Path to the PDF file.
        file: PathBuf,

        /// TOC page range (1-based, e.g. "3-5") to parse for page references.
        #[arg(long)]
        toc_pages: Option<String>,

        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Recover PDF outline bookmarks using font-size heuristics.
    ///
    /// Useful for PDFs that have no /Outlines and no /PageLabels (e.g. LaTeX
    /// books with stripped metadata).  Run with --dry-run first to preview
    /// what headings are detected, then re-run without it to write the fixed PDF.
    RecoverOutline {
        /// Path to the PDF file to analyse.
        file: PathBuf,

        /// Write the fixed PDF to a new path instead of overwriting the input.
        #[arg(long)]
        output: Option<PathBuf>,

        /// Preview detected headings without modifying any file.
        #[arg(long)]
        dry_run: bool,

        /// Font-size ratio above body text required to classify text as a heading.
        /// 1.2 means any text 20 % larger than the most common size is a heading.
        #[arg(long, default_value_t = 1.2)]
        min_font_ratio: f64,

        /// Maximum heading depth to inject (1 = chapter-level only, 2 = chapters + sections).
        #[arg(long, default_value_t = 2)]
        depth: u32,

        /// TOC page range (1-based, e.g. "3-5") for targeted heading extraction.
        #[arg(long)]
        toc_pages: Option<String>,

        /// Skip outline injection (preview only, like --dry-run but still runs
        /// the full pipeline for JSON output).
        #[arg(long)]
        no_inject: bool,

        /// Minimum fuzzy-match similarity (0.0–1.0) for heading verification.
        #[arg(long, default_value_t = 0.6)]
        fuzzy_threshold: f64,

        /// Output the full pipeline result as JSON.
        #[arg(long)]
        json: bool,

        /// Path to a reference PDF whose /Outlines provide seed chapter titles.
        /// The reference PDF must have a working outline (verify with `arcane outline`).
        /// Mutually exclusive with --seed-file.
        #[arg(long, value_name = "PDF", conflicts_with = "seed_file")]
        seed_pdf: Option<PathBuf>,

        /// Path to a JSON seed file with known chapter titles and page numbers.
        /// Format: [{"title": "...", "page": N, "depth": D}, ...]
        /// Pages are 1-based logical page numbers matching `arcane outline` display.
        #[arg(long, value_name = "JSON")]
        seed_file: Option<PathBuf>,

        /// Page-search tolerance window (±N pages) when locating seed titles
        /// in the target PDF.  Increase if chapters may be shifted by more than
        /// the default.
        #[arg(long, default_value_t = 5)]
        seed_tolerance: u32,
    },

    /// Show the outline (bookmarks) and page labels of a PDF file.
    Outline {
        /// Path to the PDF file.
        file: PathBuf,

        /// Maximum depth of outline entries to display.
        #[arg(long, default_value_t = 10)]
        depth: u32,
    },

    /// Remove a source from a project, or an entire project.
    Remove {
        /// Project name.
        project: String,

        /// Source title to remove. If omitted, removes the entire project.
        source: Option<String>,
    },

    /// Classify a PDF as text-based or scanned (image-only).
    ///
    /// Inspects every page for text-showing vs image-placing operators and
    /// reports the overall document type plus per-page breakdown.
    Probe {
        /// Path to the PDF file.
        file: PathBuf,

        /// Output as JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },

    /// Encrypt a PDF with a password.
    Protect {
        /// Input PDF file.
        input: PathBuf,

        /// Password for encryption.
        #[arg(long)]
        password: String,

        /// Output PDF file (defaults to overwriting input).
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Decrypt a password-protected PDF.
    Unlock {
        /// Input encrypted PDF file.
        input: PathBuf,

        /// Password for decryption.
        #[arg(long)]
        password: String,

        /// Output PDF file (defaults to overwriting input).
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Download OCR model files and runtime libraries to ~/Arcane/models/.
    ///
    /// Downloads PaddleOCR v5 models, ONNX Runtime, and PDFium for the
    /// current platform.  After this command, `arcane recover-outline` will
    /// auto-detect the models — no environment variables needed.
    InitOcr {
        /// Override the models directory (default: ~/Arcane/models/).
        #[arg(long)]
        models_dir: Option<PathBuf>,

        /// Skip downloading runtime libraries (onnxruntime, pdfium).
        #[arg(long)]
        skip_runtime: bool,

        /// Force re-download even if files already exist.
        #[arg(long)]
        force: bool,
    },

    /// OCR operations — run OCR on PDF pages or manage the background OCR worker.
    ///
    /// Requires `cargo build --features ocr` and `arcane init-ocr`.
    Ocr {
        #[command(subcommand)]
        cmd: OcrCommand,
    },

    /// Correlate detected chapter headings with TOC entries to find the
    /// physical-to-logical page offset.
    ///
    /// Uses RANSAC-style consensus: for every heading × TOC-entry pair
    /// whose title similarity exceeds `--threshold`, computes the candidate
    /// offset delta.  The most-voted delta is the consensus offset.
    SyncPages {
        /// Path to the PDF file.
        file: PathBuf,

        /// TOC page range (1-based, e.g. "14-20"). Auto-detected if omitted.
        #[arg(long)]
        toc_pages: Option<String>,

        /// Minimum normalised Levenshtein similarity for a heading↔TOC match.
        #[arg(long, default_value_t = 0.6)]
        threshold: f64,

        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Internal: run the OCR worker server loop (not intended for direct use).
    #[command(hide = true)]
    WorkerServe {
        /// Seconds of inactivity before the worker shuts itself down.
        #[arg(long)]
        idle_timeout_secs: Option<u64>,
    },
}

/// OCR sub-commands, invoked as `arcane ocr <sub-command>`.
#[derive(Subcommand)]
pub enum OcrCommand {
    /// Initialise OCR runtime by validating models and warming the worker.
    ///
    /// If no worker is running, this command starts one, verifies it is
    /// responsive, and stops it immediately.
    Init,

    /// Run OCR on a page range and output the recognised text.
    ///
    /// If a persistent OCR worker is running (`arcane ocr start`), the
    /// request is routed to it so the ~5 s model-load cost is only paid once.
    /// Requires `cargo build --features ocr` and `arcane init-ocr`.
    Run {
        /// Path to the PDF file.
        file: PathBuf,

        /// Page range to OCR (1-based, e.g. "1-5" or "14-20").
        #[arg(long)]
        pages: String,

        /// Render DPI (higher = more accurate but slower).
        #[arg(long, default_value_t = 150)]
        dpi: u32,

        /// Output as JSON (includes coordinates and confidence scores).
        #[arg(long)]
        json: bool,
    },

    /// Start the persistent OCR worker in the background.
    ///
    /// Loads ONNX Runtime and PaddleOCR models once, then serves requests
    /// from `arcane ocr run` and pipeline commands (e.g. `recover-outline`)
    /// via a TCP loopback socket — eliminating the ~5 s model-load overhead
    /// on every call.  Requires `cargo build --features ocr` and `arcane init-ocr`.
    Start {
        /// Shut the worker down automatically after N seconds of inactivity.
        #[arg(long)]
        idle_timeout_secs: Option<u64>,
    },

    /// Stop the running OCR worker gracefully.
    Stop,

    /// Show the status of the OCR worker (running state, PID, port, uptime).
    Status,

    /// Restart the OCR worker (stops if running, then starts fresh).
    Restart {
        /// Shut the worker down automatically after N seconds of inactivity.
        #[arg(long)]
        idle_timeout_secs: Option<u64>,
    },
}
