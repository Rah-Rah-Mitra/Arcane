//! Text extraction from PDF files.
//!
//! Provides a `PageText` struct and extraction functions that will be used
//! by the search indexer to build the tantivy full-text index.
//!
//! Currently uses `lopdf` for basic text extraction. When `oxidize-pdf` or
//! `pdf-extract` is added in a future phase, this module will be upgraded
//! for higher-quality extraction.

use std::path::Path;

use anyhow::{Context, Result};
use lopdf::Document;

/// Text content extracted from a single PDF page.
#[derive(Debug, Clone)]
pub struct PageText {
    /// 0-based physical page index.
    pub page_index: u32,
    /// Extracted text content.
    pub text: String,
    /// Number of whitespace-delimited words.
    pub word_count: usize,
}

/// Extract text from all pages of a PDF.
///
/// Returns a `Vec<PageText>` ordered by physical page index.
pub fn extract_all(path: &Path) -> Result<Vec<PageText>> {
    let doc = Document::load(path)
        .with_context(|| format!("failed to open PDF for text extraction: {}", path.display()))?;

    let pages = doc.get_pages();
    let mut result = Vec::with_capacity(pages.len());

    for &page_num in pages.keys() {
        let text = doc.extract_text(&[page_num]).unwrap_or_default();
        let word_count = text.split_whitespace().count();

        result.push(PageText {
            page_index: page_num.saturating_sub(1), // convert 1-based → 0-based
            text,
            word_count,
        });
    }

    // Sort by page index in case the BTreeMap iteration wasn't ordered.
    result.sort_by_key(|p| p.page_index);

    Ok(result)
}
