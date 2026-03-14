//! Typed error hierarchy for Arcane.
//!
//! Library modules use [`thiserror`] for structured, matchable errors.
//! Application-level code (CLI handlers) wraps these via [`anyhow`] for
//! ergonomic context propagation.

use thiserror::Error;

// ---------------------------------------------------------------------------
// Top-level application error
// ---------------------------------------------------------------------------

/// Umbrella error that collects all domain-specific errors.
#[derive(Error, Debug)]
pub enum ArcaneError {
    #[error("PDF processing error: {0}")]
    Pdf(#[from] PdfError),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Search error: {0}")]
    Search(#[from] SearchError),

    #[error("Watcher error: {0}")]
    Watcher(#[from] WatcherError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// PDF errors
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum PdfError {
    #[error("failed to open PDF at {path}: {source}")]
    OpenFailed { path: String, source: anyhow::Error },

    #[error("no chapter boundaries found in {path}")]
    NoChapters { path: String },

    #[error("page range {start}-{end} exceeds document length {total}")]
    InvalidRange { start: u32, end: u32, total: u32 },

    #[error("text extraction failed: {0}")]
    TextExtraction(String),

    #[error("merge failed: {0}")]
    MergeFailed(String),

    #[error("PDF operation failed: {0}")]
    OperationFailed(String),
}

// ---------------------------------------------------------------------------
// Storage errors
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("project '{name}' not found")]
    ProjectNotFound { name: String },

    #[error("source '{title}' not found in project '{project}'")]
    SourceNotFound { title: String, project: String },

    #[error("filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),

    #[error("migration failed at version {version}: {reason}")]
    MigrationFailed { version: String, reason: String },

    #[error("serialization error: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// Search errors
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("index error: {0}")]
    IndexError(String),

    #[error("query parse error: {0}")]
    QueryError(String),
}

// ---------------------------------------------------------------------------
// Watcher errors
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum WatcherError {
    #[error("watcher error: {0}")]
    WatchFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
