//! CLI layer — clap command definitions and dispatch.
//!
//! # Three-tier command architecture
//!
//! ```text
//! Tier 0 — Base PDF operations   arcane pdf <op>
//!   merge, split, rotate, protect, unlock, inject-outlines, extract-pages
//!
//! Tier 1 — Analysis / inspection  arcane analyze <op>
//!   probe, outline, layout, offset, sync-pages
//!
//! Tier 2 — Project workflows      arcane <workflow>
//!   chunk, recover-outline, recover, recover-project, process-toc,
//!   search, reindex, freq, tui, watch
//!
//! Project management              arcane <cmd>
//!   new, list, show, add, remove, tag, untag, list-chunks
//! ```
//!
//! Workflow compositions are documented in `.claude/commands/`.
//! Module efficiency agents are defined in `.claude/agents/`.

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
    // ── Project management ────────────────────────────────────────────────
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

        /// 1-based start page of the table of contents.
        #[arg(long)]
        toc_start_page: Option<u32>,

        /// 1-based end page of the table of contents.
        #[arg(long)]
        toc_end_page: Option<u32>,

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

    /// Remove a source from a project, or an entire project.
    Remove {
        /// Project name.
        project: String,

        /// Source title to remove. If omitted, removes the entire project.
        source: Option<String>,
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

    /// List chunk files for a source in a project.
    ListChunks {
        /// Project name.
        project: String,

        /// Source title. If omitted, lists chunks for all sources.
        source: Option<String>,
    },

    // ── Base PDF operations (Tier 0) ──────────────────────────────────────
    /// Atomic PDF file operations: merge, split, rotate, encrypt, inject-outlines, extract-pages.
    ///
    /// These are the composable building blocks for higher-level workflows.
    /// Each operation is lossless (no re-encoding).
    Pdf {
        #[command(subcommand)]
        op: PdfCommands,
    },

    // ── Analysis / inspection (Tier 1) ────────────────────────────────────
    /// PDF analysis and inspection: probe, outline, layout, offset, sync-pages.
    ///
    /// Use these to understand a PDF's structure before running recovery or chunking.
    Analyze {
        #[command(subcommand)]
        op: AnalyzeCommands,
    },

    // ── Workflow commands (Tier 2) ────────────────────────────────────────
    /// Split textbook sources into per-chapter PDFs.
    ///
    /// Workflow: detect_boundaries → chunk_pdf → inject_outlines (if chapter_map set).
    /// See `.claude/commands/chunk-textbook.md` for step-by-step guidance.
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

        /// Only chunk a specific source (by title).
        #[arg(long)]
        source: Option<String>,
    },

    /// Recover PDF outline bookmarks using font-size heuristics.
    ///
    /// Workflow: probe → layout::analyze → offset::calculate → heuristics::inject.
    /// See `.claude/commands/recover-outline-heuristic.md` for step-by-step guidance.
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
        #[arg(long, default_value_t = 1.2)]
        min_font_ratio: f64,

        /// Maximum heading depth to inject (1 = chapter-level only, 2 = chapters + sections).
        #[arg(long, default_value_t = 2)]
        depth: u32,

        /// TOC page range (1-based, e.g. "3-5") for targeted heading extraction.
        #[arg(long)]
        toc_pages: Option<String>,

        /// Skip outline injection (preview only).
        #[arg(long)]
        no_inject: bool,

        /// Minimum fuzzy-match similarity (0.0–1.0) for heading verification.
        #[arg(long, default_value_t = 0.6)]
        fuzzy_threshold: f64,

        /// Output the full pipeline result as JSON.
        #[arg(long)]
        json: bool,

        /// Path to a reference PDF whose /Outlines provide seed chapter titles.
        #[arg(long, value_name = "PDF", conflicts_with = "seed_file")]
        seed_pdf: Option<PathBuf>,

        /// Path to a JSON seed file with known chapter titles and page numbers.
        #[arg(long, value_name = "JSON")]
        seed_file: Option<PathBuf>,

        /// Page-search tolerance window (±N pages) when locating seed titles.
        #[arg(long, default_value_t = 5)]
        seed_tolerance: u32,

        /// Offset search range (±N) for auto-detecting the physical page offset.
        #[arg(long, default_value_t = 50)]
        offset_tolerance: u32,

        /// First TOC page (1-based). Alternative to `--toc-pages` range string.
        #[arg(long)]
        toc_start_page: Option<u32>,

        /// Last TOC page (1-based). Alternative to `--toc-pages` range string.
        #[arg(long)]
        toc_end_page: Option<u32>,

        /// Physical PDF page number (1-based) where the book's printed page 1 begins.
        #[arg(long, value_name = "PDF_PAGE")]
        page_one: Option<u32>,

        /// Per-segment page pivot in the form LOGICAL:PHYSICAL (both 1-based).
        #[arg(long, value_name = "LOGICAL:PHYSICAL", value_parser = crate::cli::commands::parse_anchor_pair)]
        anchor: Vec<(u32, u32)>,
    },

    /// OCR TOC pages of a PDF and output a seed JSON for outline recovery.
    ///
    /// Workflow: bridge::extract_pages → bridge::client::parse_toc_entries.
    /// See `.claude/commands/recover-outline-bridge.md`.
    ProcessToc {
        /// Path to the source PDF.
        pdf: PathBuf,

        /// 1-based physical TOC page range, e.g. "7-18".
        #[arg(long)]
        toc_pages: String,

        /// Arcane-PP server URL.
        #[arg(long, default_value = "http://localhost:5000")]
        server: String,

        /// Write seed JSON to this file instead of stdout.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,

        /// Preferred injection depth for downstream outline recovery.
        #[arg(long, default_value_t = 2)]
        depth: u32,
    },

    /// Full bridge pipeline: extract TOC pages, parse entries, and recover outline.
    ///
    /// Workflow: process-toc → recover-outline (seeded).
    /// See `.claude/commands/recover-outline-bridge.md`.
    Recover {
        /// Path to the source PDF.
        pdf: PathBuf,

        /// 1-based physical TOC page range, e.g. "7-18".
        #[arg(long)]
        toc_pages: String,

        /// Arcane-PP server URL.
        #[arg(long, default_value = "http://localhost:5000")]
        server: String,

        /// Output PDF path. Defaults to overwriting the input file.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,

        /// Maximum hierarchy depth to inject into outline bookmarks.
        #[arg(long, default_value_t = 2)]
        depth: u32,

        /// Preview only — do not modify any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Batch outline recovery for a project's sources.
    ///
    /// Workflow: [for each source needing recovery] recover.
    /// See `.claude/commands/recover-outline-bridge.md`.
    RecoverProject {
        /// Arcane project name to process.
        #[arg(long, default_value = "Computer-Vision")]
        project: String,

        /// Arcane-PP server URL.
        #[arg(long, default_value = "http://localhost:5000")]
        server: String,

        /// Maximum hierarchy depth to inject into outline bookmarks.
        #[arg(long, default_value_t = 2)]
        depth: u32,

        /// Preview only — do not modify any file.
        #[arg(long)]
        dry_run: bool,

        /// Arcane data root containing projects.json. Defaults to ~/Arcane.
        #[arg(long)]
        arcane_data: Option<PathBuf>,
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

    /// Generate a frequency dictionary for a project.
    Freq {
        /// Project name.
        project: String,

        /// Output file path (default: freq.txt in the project directory).
        #[arg(long)]
        output: Option<PathBuf>,

        /// Maximum number of entries to include (0 = unlimited).
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },

    /// Launch the interactive terminal UI.
    Tui,

    /// Watch a project directory for new PDFs.
    Watch {
        /// Project name to watch.
        project: String,
    },

    // ── Legacy flat commands (deprecated — use `arcane pdf` or `arcane analyze`) ──
    /// [Deprecated] Use `arcane pdf merge`. Merge multiple PDF files into one.
    #[command(hide = true)]
    Merge {
        output: PathBuf,
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },

    /// [Deprecated] Use `arcane pdf split`. Split a PDF into multiple files by page ranges.
    #[command(hide = true)]
    Split {
        input: PathBuf,
        #[arg(long, default_value = ".")]
        output_dir: PathBuf,
        #[arg(required = true)]
        ranges: Vec<String>,
    },

    /// [Deprecated] Use `arcane pdf rotate`. Rotate pages in a PDF.
    #[command(hide = true)]
    Rotate {
        input: PathBuf,
        #[arg(long, default_value_t = 90)]
        degrees: i32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        pages: Vec<u32>,
    },

    /// [Deprecated] Use `arcane pdf protect`. Encrypt a PDF with a password.
    #[command(hide = true)]
    Protect {
        input: PathBuf,
        #[arg(long)]
        password: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// [Deprecated] Use `arcane pdf unlock`. Decrypt a password-protected PDF.
    #[command(hide = true)]
    Unlock {
        input: PathBuf,
        #[arg(long)]
        password: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// [Deprecated] Use `arcane analyze probe`. Classify a PDF as text-based or scanned.
    #[command(hide = true)]
    Probe {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// [Deprecated] Use `arcane analyze outline`. Show the outline of a PDF file.
    #[command(hide = true)]
    Outline {
        file: PathBuf,
        #[arg(long, default_value_t = 10)]
        depth: u32,
    },

    /// [Deprecated] Use `arcane analyze layout`. Detect layout structure.
    #[command(hide = true)]
    DetectLayout {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        pages: Option<String>,
    },

    /// [Deprecated] Use `arcane analyze offset`. Calculate the logical-to-physical page offset.
    #[command(hide = true)]
    FindOffset {
        file: PathBuf,
        #[arg(long)]
        toc_pages: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// [Deprecated] Use `arcane analyze sync-pages`. RANSAC heading↔TOC offset consensus.
    #[command(hide = true)]
    SyncPages {
        file: PathBuf,
        #[arg(long)]
        toc_pages: Option<String>,
        #[arg(long, default_value_t = 0.6)]
        threshold: f64,
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------
// Base PDF operations subcommand enum (Tier 0)
// ---------------------------------------------------------------------------

/// Atomic, lossless PDF file operations.
///
/// Each sub-command wraps a single `crate::pdf` function.  Combine these in
/// shell scripts or workflow skill files to build higher-level pipelines.
#[derive(Subcommand)]
pub enum PdfCommands {
    /// Merge multiple PDF files into one (lossless).
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

    /// Rotate pages in a PDF (lossless).
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

    /// Inject outline bookmarks from a JSON chapter-map into a PDF.
    ///
    /// JSON format: `{"18": "Chapter 1", "44": "Chapter 2"}` (0-based page → title).
    /// This is the base operation used internally by `arcane chunk`.
    InjectOutlines {
        /// Input PDF file.
        input: PathBuf,
        /// JSON file mapping 0-based page numbers to chapter titles.
        #[arg(long, value_name = "JSON")]
        chapters: PathBuf,
        /// Output PDF file (defaults to overwriting input).
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Extract a contiguous page range from a PDF into a new file.
    ///
    /// Page numbers are 1-based physical indices.
    /// This is the base operation used internally by `arcane process-toc` and `arcane recover`.
    ExtractPages {
        /// Input PDF file.
        input: PathBuf,
        /// First page to extract (1-based).
        #[arg(long)]
        start: u32,
        /// Last page to extract (1-based, inclusive).
        #[arg(long)]
        end: u32,
        /// Output PDF file.
        output: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// Analysis / inspection subcommand enum (Tier 1)
// ---------------------------------------------------------------------------

/// PDF structural analysis and inspection operations.
///
/// Use these to understand a PDF before running recovery or chunking workflows.
#[derive(Subcommand)]
pub enum AnalyzeCommands {
    /// Classify a PDF as text-based, scanned (image-only), mixed, or empty.
    ///
    /// Wraps `pdf::probe::probe`. Run this first to confirm a PDF is text-based
    /// before attempting outline recovery.
    Probe {
        /// Path to the PDF file.
        file: PathBuf,
        /// Output as JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },

    /// Show the outline (bookmarks) and page labels of a PDF file.
    ///
    /// Wraps `pdf::outlines::extract_chapters_with_depth` and
    /// `pdf::page_labels::extract_chapters_from_page_labels`.
    Outline {
        /// Path to the PDF file.
        file: PathBuf,
        /// Maximum depth of outline entries to display.
        #[arg(long, default_value_t = 10)]
        depth: u32,
    },

    /// Detect layout structure and output structural anchors.
    ///
    /// Wraps `pdf::layout::analyze_layout` (4-phase typographic pipeline).
    /// Identifies headings, TOC entries, and page numbers via font clustering.
    Layout {
        /// Path to the PDF file.
        file: PathBuf,
        /// Output as JSON (default: human-readable summary).
        #[arg(long)]
        json: bool,
        /// Only analyse specific pages (0-based range, e.g. "0-5").
        #[arg(long)]
        pages: Option<String>,
    },

    /// Calculate the logical-to-physical page offset for a PDF.
    ///
    /// Wraps `pdf::offset::calculate_offset` (PageLabels → TOC matching → page numbers).
    /// Use the result with `--page-one` in `arcane recover-outline`.
    Offset {
        /// Path to the PDF file.
        file: PathBuf,
        /// TOC page range (1-based, e.g. "3-5") for TOC-matching strategy.
        #[arg(long)]
        toc_pages: Option<String>,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },

    /// RANSAC heading↔TOC consensus offset estimation.
    ///
    /// Matches detected headings against TOC entries to find the consensus
    /// physical-to-logical page offset.  Use with `--toc-pages` for best results.
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
}
