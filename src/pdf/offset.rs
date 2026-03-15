//! Logical-to-physical page offset calculation.
//!
//! Solves the "front matter" problem: many textbooks have Roman-numeral preface
//! pages, so printed page 1 does not correspond to physical page 1 in the PDF.
//! This module determines the integer delta between printed page numbers and
//! PDF page indices using multiple strategies:
//!
//! 1. `/PageLabels` number tree (fastest, most reliable when present)
//! 2. TOC matching — compare TOC-entry page numbers against page text
//! 3. Page-number detection — find printed numbers in headers/footers

use lopdf::Document;
use serde::{Deserialize, Serialize};

use super::layout::{self, LayoutResult, PositionedText};
use super::page_labels::PageLabelResolver;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of offset calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffsetResult {
    /// The computed offset: `physical_page_index - printed_page_number`.
    /// Example: if printed page 1 is at physical index 18, offset = 18.
    pub offset: i32,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f32,
    /// Method used to determine the offset.
    pub method: OffsetMethod,
    /// Supporting evidence.
    pub evidence: Vec<OffsetEvidence>,
}

/// Method used to determine the page offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffsetMethod {
    /// Derived from the `/PageLabels` number tree.
    PageLabels,
    /// Derived from matching TOC page numbers against document text.
    TocMatching,
    /// Derived from page numbers detected in headers/footers.
    PageNumberDetection,
    /// User-supplied via `--toc-pages`.
    UserSupplied,
}

/// A single piece of evidence supporting the offset calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffsetEvidence {
    /// Physical page index where evidence was found.
    pub physical_page: u32,
    /// The detected logical (printed) page number.
    pub logical_number: u32,
    /// Text that was matched / detected.
    pub matched_text: String,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Calculate the logical-to-physical page offset.
///
/// Tries methods in priority order:
/// 1. `/PageLabels` (if present)
/// 2. TOC matching (if layout + TOC pages available)
/// 3. Page-number detection from headers/footers
///
/// Returns `None` if no offset can be determined.
pub fn calculate_offset(
    doc: &Document,
    layout: Option<&LayoutResult>,
    toc_pages: Option<(u32, u32)>,
) -> Option<OffsetResult> {
    // Strategy 1: PageLabels.
    if let Some(result) = offset_from_page_labels(doc) {
        return Some(result);
    }

    // We need positioned text for the remaining strategies.
    let positioned = if layout.is_some() {
        // Layout was already computed — re-extract positioned text.
        layout::extract_all_positioned(doc)
    } else {
        layout::extract_all_positioned(doc)
    };

    // Strategy 2: TOC matching.
    if let Some(toc_range) = toc_pages {
        let entries = parse_toc_entries(&positioned, toc_range);
        if !entries.is_empty() {
            if let Some(result) = offset_from_toc_matching(doc, &entries) {
                return Some(result);
            }
        }
    } else {
        // Try auto-detected TOC pages.
        let toc_page_indices = layout::detect_toc_pages(&positioned);
        if !toc_page_indices.is_empty() {
            let toc_start = *toc_page_indices.first().unwrap();
            let toc_end = *toc_page_indices.last().unwrap();
            let entries = parse_toc_entries(&positioned, (toc_start, toc_end));
            if !entries.is_empty() {
                if let Some(result) = offset_from_toc_matching(doc, &entries) {
                    return Some(result);
                }
            }
        }
    }

    // Strategy 3: Page-number detection.
    let page_numbers = layout::detect_page_numbers(&positioned, 792.0); // default letter height
    if page_numbers.len() >= 3 {
        return offset_from_page_numbers(&page_numbers);
    }

    None
}

// ---------------------------------------------------------------------------
// Strategy 1: PageLabels
// ---------------------------------------------------------------------------

/// Derive offset from the `/PageLabels` number tree.
fn offset_from_page_labels(doc: &Document) -> Option<OffsetResult> {
    let resolver = PageLabelResolver::from_document(doc).ok()?;
    let offset = resolver.arabic_offset()?;

    let content_start = resolver.content_start()?;
    let evidence = vec![OffsetEvidence {
        physical_page: content_start,
        logical_number: 1,
        matched_text: format!(
            "PageLabels: Arabic numbering starts at physical page {}",
            content_start
        ),
    }];

    Some(OffsetResult {
        offset,
        confidence: 0.95,
        method: OffsetMethod::PageLabels,
        evidence,
    })
}

// ---------------------------------------------------------------------------
// Strategy 2: TOC matching
// ---------------------------------------------------------------------------

/// Parse TOC entries from positioned text on specified pages.
///
/// Extracts `(title, printed_page_number)` pairs by looking for lines
/// matching the "Title ... number" pattern.
pub fn parse_toc_entries(
    positioned: &[PositionedText],
    toc_pages: (u32, u32),
) -> Vec<(String, u32)> {
    let mut entries = Vec::new();

    for pt in positioned {
        if pt.page_index < toc_pages.0 || pt.page_index > toc_pages.1 {
            continue;
        }

        let trimmed = pt.text.trim();
        if let Some(parsed) = parse_toc_line(trimmed) {
            entries.push(parsed);
        }
    }

    entries
}

/// Try to parse a single line as a TOC entry.
/// Returns `Some((title, page_number))` if successful.
fn parse_toc_line(text: &str) -> Option<(String, u32)> {
    let trimmed = text.trim();
    if trimmed.len() < 3 {
        return None;
    }

    // Find the last word — it should be a page number.
    let last_word = trimmed.split_whitespace().last()?;
    if last_word.len() > 4 || !last_word.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let page_num: u32 = last_word.parse().ok()?;
    if page_num == 0 {
        return None;
    }

    // Extract the title: everything before the page number.
    let title_end = trimmed.len() - last_word.len();
    let title = trimmed[..title_end]
        .trim_end_matches(['.', ' ', '\t', '\u{a0}'])
        .trim()
        .to_string();

    if title.len() >= 2 {
        Some((title, page_num))
    } else {
        None
    }
}

/// Determine offset by matching TOC entries against page text.
///
/// For each TOC entry `(title, printed_page)`, tries candidate offsets 0..50.
/// The offset where the most titles fuzzy-match text on the candidate page wins.
fn offset_from_toc_matching(doc: &Document, toc_entries: &[(String, u32)]) -> Option<OffsetResult> {
    if toc_entries.is_empty() {
        return None;
    }

    let total_pages = doc.get_pages().len() as u32;

    // Pre-extract text for each page (using lopdf's extract_text).
    let page_texts: Vec<String> = (0..total_pages)
        .map(|i| {
            doc.extract_text(&[i + 1]) // 1-based
                .unwrap_or_default()
        })
        .collect();

    let mut best_offset: i32 = 0;
    let mut best_matches: u32 = 0;
    let mut best_evidence: Vec<OffsetEvidence> = Vec::new();

    // Try offsets from -5 to 50.
    for candidate_offset in -5..=50_i32 {
        let mut matches = 0u32;
        let mut evidence = Vec::new();

        for (title, printed_page) in toc_entries {
            let physical = *printed_page as i32 + candidate_offset - 1; // 0-based
            if physical < 0 || physical >= total_pages as i32 {
                continue;
            }

            let page_text = &page_texts[physical as usize];
            if page_text.is_empty() {
                continue;
            }

            // Fuzzy match: check if the title appears in the page text.
            let similarity = best_substring_similarity(title, page_text);
            if similarity >= 0.6 {
                matches += 1;
                evidence.push(OffsetEvidence {
                    physical_page: physical as u32,
                    logical_number: *printed_page,
                    matched_text: title.clone(),
                });
            }
        }

        if matches > best_matches {
            best_matches = matches;
            best_offset = candidate_offset;
            best_evidence = evidence;
        }
    }

    if best_matches >= 2 {
        let confidence = (best_matches as f32 / toc_entries.len() as f32).min(1.0) * 0.9;
        Some(OffsetResult {
            offset: best_offset,
            confidence,
            method: OffsetMethod::TocMatching,
            evidence: best_evidence,
        })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Strategy 3: Page-number detection
// ---------------------------------------------------------------------------

/// Determine offset from detected page numbers in headers/footers.
///
/// Uses consensus: the most common `(physical_page - detected_number)` wins.
fn offset_from_page_numbers(detected: &[(u32, u32)]) -> Option<OffsetResult> {
    use std::collections::HashMap;

    if detected.is_empty() {
        return None;
    }

    // Count occurrences of each offset.
    let mut offset_counts: HashMap<i32, Vec<OffsetEvidence>> = HashMap::new();
    for &(physical, logical) in detected {
        let offset = physical as i32 - logical as i32 + 1; // +1 because physical is 0-based
        offset_counts
            .entry(offset)
            .or_default()
            .push(OffsetEvidence {
                physical_page: physical,
                logical_number: logical,
                matched_text: format!("page number {} at physical page {}", logical, physical),
            });
    }

    // Find the offset with the most votes.
    let (best_offset, evidence) = offset_counts.into_iter().max_by_key(|(_, v)| v.len())?;

    let vote_count = evidence.len();
    if vote_count < 3 {
        return None;
    }

    let confidence = (vote_count as f32 / detected.len() as f32).min(1.0) * 0.8;

    Some(OffsetResult {
        offset: best_offset,
        confidence,
        method: OffsetMethod::PageNumberDetection,
        evidence,
    })
}

// ---------------------------------------------------------------------------
// Fuzzy matching helpers
// ---------------------------------------------------------------------------

/// Find the best fuzzy-match similarity between `needle` and any substring
/// of `haystack` of similar length.
///
/// Uses normalized Levenshtein distance from the `strsim` crate.
fn best_substring_similarity(needle: &str, haystack: &str) -> f64 {
    let needle_lower = needle.to_lowercase();
    let haystack_lower = haystack.to_lowercase();

    // Direct check: if the haystack contains the needle exactly.
    if haystack_lower.contains(&needle_lower) {
        return 1.0;
    }

    // Check each line of the haystack.
    let mut best = 0.0_f64;
    for line in haystack_lower.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let sim = strsim::normalized_levenshtein(&needle_lower, line);
        if sim > best {
            best = sim;
        }

        // Also check if the line starts with or contains something close.
        if line.len() > needle_lower.len() {
            let prefix = &line[..needle_lower.len().min(line.len())];
            let sim2 = strsim::normalized_levenshtein(&needle_lower, prefix);
            if sim2 > best {
                best = sim2;
            }
        }
    }

    best
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_toc_line_valid() {
        assert_eq!(
            parse_toc_line("Introduction .... 15"),
            Some(("Introduction".into(), 15))
        );
        assert_eq!(
            parse_toc_line("Chapter 1  42"),
            Some(("Chapter 1".into(), 42))
        );
        assert_eq!(parse_toc_line("Methods 100"), Some(("Methods".into(), 100)));
    }

    #[test]
    fn parse_toc_line_invalid() {
        assert_eq!(parse_toc_line("42"), None); // too short for title
        assert_eq!(parse_toc_line("ab"), None); // no number
        assert_eq!(parse_toc_line(""), None);
    }

    #[test]
    fn best_substring_similarity_exact() {
        let sim =
            best_substring_similarity("Introduction", "Chapter 1: Introduction to Algorithms");
        assert!(sim >= 0.9, "expected high similarity, got {sim}");
    }

    #[test]
    fn best_substring_similarity_low() {
        let sim = best_substring_similarity("Appendix", "Chapter 1: Introduction to Algorithms");
        assert!(sim < 0.6, "expected low similarity, got {sim}");
    }

    #[test]
    fn offset_from_page_numbers_consensus() {
        // 5 pages all agree: physical 18 = page 1, physical 19 = page 2, etc.
        let detected: Vec<(u32, u32)> = (0..5).map(|i| (18 + i, 1 + i)).collect();
        let result = offset_from_page_numbers(&detected).unwrap();
        assert_eq!(result.offset, 18);
        assert_eq!(result.method, OffsetMethod::PageNumberDetection);
    }

    #[test]
    fn offset_from_page_numbers_insufficient_evidence() {
        // Only 2 data points — not enough for confidence.
        let detected = vec![(18, 1), (19, 2)];
        assert!(offset_from_page_numbers(&detected).is_none());
    }

    #[test]
    fn parse_toc_entries_filters_by_page_range() {
        let positioned = vec![
            PositionedText {
                page_index: 2,
                x: 72.0,
                y: 700.0,
                font_size: 12.0,
                font_key: "F1".into(),
                text: "Chapter 1 .... 15".into(),
            },
            PositionedText {
                page_index: 5, // outside range
                x: 72.0,
                y: 700.0,
                font_size: 12.0,
                font_key: "F1".into(),
                text: "Chapter 2 .... 30".into(),
            },
        ];

        let entries = parse_toc_entries(&positioned, (2, 3));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "Chapter 1");
        assert_eq!(entries[0].1, 15);
    }
}
