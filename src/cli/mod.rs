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
}
