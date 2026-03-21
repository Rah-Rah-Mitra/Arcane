//! Source metadata and the [`Source`] trait.
//!
//! Defines the [`SourceMeta`] record (persisted in the database), the
//! [`Source`] trait that every source type must implement, and the two
//! concrete source types: [`Textbook`] and [`Report`].

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 1-based page range where the table of contents appears in the source PDF.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentsPageRange {
    pub start: u32,
    pub end: u32,
}

// ---------------------------------------------------------------------------
// Source metadata (persisted)
// ---------------------------------------------------------------------------

/// Serialisable record for each source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    /// Display name / title of the source.
    pub title: String,

    /// Absolute path to the original PDF file.
    pub path: PathBuf,

    /// `true` when this source is a large document that should be split into
    /// per-chapter PDFs; `false` for cheat-sheets, reports, etc.
    pub needs_chunking: bool,

    /// Maps **physical** (0-based) page indices to logical chapter names.
    ///
    /// Example: `{0: "Front Matter", 12: "Chapter 1 — Introduction", …}`
    #[serde(default)]
    pub chapter_map: HashMap<u32, String>,

    /// When `chapter_map` is empty and `needs_chunking` is `true`, the user
    /// may supply the physical page index where printed page 1 starts.  This
    /// allows the engine to compute `offset = physical - logical`.
    #[serde(default)]
    pub start_page_physical: Option<u32>,

    /// Outline extraction depth used during the last chunking operation.
    /// `None` if never chunked, or if chunked without using outlines.
    #[serde(default)]
    pub depth: Option<u32>,

    /// Total page count of the source PDF.
    /// `None` if not yet determined.
    #[serde(default)]
    pub page_count: Option<u32>,

    /// Optional 1-based page range containing the document's table of contents.
    #[serde(default)]
    pub contents_page_range: Option<ContentsPageRange>,
}

impl SourceMeta {
    /// Convenience constructor for a source that does **not** require chunking.
    pub fn report(title: impl Into<String>, path: PathBuf) -> Self {
        Self {
            title: title.into(),
            path,
            needs_chunking: false,
            chapter_map: HashMap::new(),
            start_page_physical: None,
            depth: None,
            page_count: None,
            contents_page_range: None,
        }
    }

    /// Convenience constructor for a source that **does** require chunking.
    pub fn textbook(
        title: impl Into<String>,
        path: PathBuf,
        chapter_map: HashMap<u32, String>,
        start_page_physical: Option<u32>,
    ) -> Self {
        Self {
            title: title.into(),
            path,
            needs_chunking: true,
            chapter_map,
            start_page_physical,
            depth: None,
            page_count: None,
            contents_page_range: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Source trait
// ---------------------------------------------------------------------------

/// Behaviour that every source type must implement.
pub trait Source {
    /// Split the source into per-chapter PDFs inside `chunks_dir`.
    ///
    /// `depth` controls how many levels of the outline tree to use for
    /// boundary detection (1 = top-level only).
    ///
    /// Implementations are **idempotent**: if the chunks directory is already
    /// populated they should return early without re-processing.
    ///
    /// Returns `(page_count, chunk_count)` on success.
    fn chunk(&self, chunks_dir: &std::path::Path, depth: u32) -> Result<(u32, usize)>;

    /// Placeholder for future YouTube transcript / video-lecture integration.
    #[allow(dead_code)]
    #[allow(unused_variables)]
    fn youtube(&self, url: &str) -> Result<()> {
        anyhow::bail!("YouTube integration is not yet implemented for this source type")
    }
}

// ---------------------------------------------------------------------------
// Textbook — implements chunking
// ---------------------------------------------------------------------------

/// A large PDF (textbook / lecture notes) that should be split into individual
/// chapter files.
pub struct Textbook {
    pub meta: SourceMeta,
}

impl Textbook {
    pub fn new(meta: SourceMeta) -> Self {
        Self { meta }
    }
}

impl Source for Textbook {
    fn chunk(&self, chunks_dir: &std::path::Path, depth: u32) -> Result<(u32, usize)> {
        crate::pdf::engine::chunk_pdf(&self.meta, chunks_dir, depth)
    }
}

// ---------------------------------------------------------------------------
// Report — skips chunking
// ---------------------------------------------------------------------------

/// A short document (report, cheat-sheet, paper) that is stored as-is without
/// being split.
pub struct Report {
    pub meta: SourceMeta,
}

impl Report {
    pub fn new(meta: SourceMeta) -> Self {
        Self { meta }
    }
}

impl Source for Report {
    fn chunk(&self, _chunks_dir: &std::path::Path, _depth: u32) -> Result<(u32, usize)> {
        tracing::info!(
            "'{}' is a Report — chunking is not required.",
            self.meta.title
        );
        // For reports, we return 0 page count and 0 chunks since they don't get chunked
        // The caller can handle getting actual page count if needed
        Ok((0, 0))
    }
}

// ---------------------------------------------------------------------------
// Factory helper
// ---------------------------------------------------------------------------

/// Build the correct [`Source`] implementation from a [`SourceMeta`] record.
pub fn build_source(meta: SourceMeta) -> Box<dyn Source> {
    if meta.needs_chunking {
        Box::new(Textbook::new(meta))
    } else {
        Box::new(Report::new(meta))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn textbook_meta_needs_chunking() {
        let meta = SourceMeta::textbook(
            "SICP",
            PathBuf::from("/tmp/sicp.pdf"),
            HashMap::new(),
            Some(10),
        );
        assert!(meta.needs_chunking);
        assert_eq!(meta.start_page_physical, Some(10));
    }

    #[test]
    fn build_source_dispatch() {
        let report_meta = SourceMeta::report("Notes", PathBuf::from("/tmp/notes.pdf"));
        let source = build_source(report_meta);
        // Report::chunk should succeed without touching the filesystem
        let result = source.chunk(std::path::Path::new("/tmp"), 1);
        assert!(result.is_ok());
    }

    #[test]
    fn source_meta_serialise_round_trip() {
        let mut chapters = HashMap::new();
        chapters.insert(0u32, "Front Matter".to_string());
        chapters.insert(12u32, "Chapter 1".to_string());

        let meta = SourceMeta::textbook(
            "Test Book",
            PathBuf::from("/tmp/book.pdf"),
            chapters.clone(),
            Some(5),
        );

        let json = serde_json::to_string(&meta).expect("serialise");
        let back: SourceMeta = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.title, "Test Book");
        assert_eq!(back.chapter_map.len(), chapters.len());
        assert_eq!(back.start_page_physical, Some(5));
    }
}
