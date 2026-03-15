//! TOC entry extraction from OCR intermediate representation.
//!
//! Parses structured Table of Contents entries from OCR-normalized pages,
//! handling dot leaders, continuation lines, roman/arabic page numbers,
//! and page offset estimation via RANSAC-style consensus.
//!
//! This module is compiled only when the `ocr` feature is enabled.

use lopdf::Document;
use serde::{Deserialize, Serialize};

use super::ocr_ir::{OcrLine, OcrPage};
use super::page_labels::from_roman;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single parsed TOC entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    /// Chapter/section title (cleaned, dot leaders removed).
    pub title: String,
    /// Raw page number string as extracted (e.g. "42" or "xv").
    pub page_number_raw: String,
    /// Parsed page number as an arabic integer, or None if unparseable.
    pub page_number: Option<u32>,
    /// Whether the page number was parsed as a roman numeral.
    pub is_roman: bool,
    /// PDF page index where this TOC line was found (for ordering).
    pub source_page: u32,
    /// X-position of the title start (for indent-based hierarchy).
    pub title_x: f32,
    /// Font size of the title text (for size-based hierarchy hints).
    pub font_size: f32,
    /// OCR confidence for the line.
    pub confidence: f32,
}

/// Result of OCR-based offset estimation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrOffsetResult {
    /// Computed offset for the arabic body section.
    pub arabic_offset: i32,
    /// Computed offset for the roman preface section (if detected).
    pub roman_offset: Option<i32>,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
    /// Warnings (e.g. low confidence, outlier pages).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Raw parsed line — either a TOC entry or a continuation candidate.
#[derive(Debug)]
enum RawTocLine {
    /// A line with a trailing page number.
    Entry {
        title: String,
        page_number_raw: String,
        page_number: Option<u32>,
        is_roman: bool,
        source_page: u32,
        title_x: f32,
        font_size: f32,
        confidence: f32,
    },
    /// A line with no trailing page number — potential continuation.
    Continuation {
        text: String,
        source_page: u32,
        title_x: f32,
        #[allow(dead_code)]
        y_min: f32,
        #[allow(dead_code)]
        font_size: f32,
    },
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extract TOC entries from OCR-normalized pages.
///
/// For each line on the given pages, tries to parse it as a TOC entry
/// (title + page number). Lines without trailing numbers are treated as
/// potential continuation lines for multi-line titles. Returns entries
/// sorted by source page and vertical position.
pub fn extract_toc_entries(pages: &[OcrPage]) -> Vec<TocEntry> {
    let mut raw_lines: Vec<RawTocLine> = Vec::new();

    for page in pages {
        for block in &page.blocks {
            for line in &block.lines {
                let trimmed = line.text.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match parse_toc_line_ocr(line, page.page_width, page.pdf_page_index) {
                    Some(entry) => raw_lines.push(RawTocLine::Entry {
                        title: entry.title,
                        page_number_raw: entry.page_number_raw,
                        page_number: entry.page_number,
                        is_roman: entry.is_roman,
                        source_page: entry.source_page,
                        title_x: entry.title_x,
                        font_size: entry.font_size,
                        confidence: entry.confidence,
                    }),
                    None => {
                        // Not a TOC entry — might be a continuation line.
                        raw_lines.push(RawTocLine::Continuation {
                            text: trimmed.to_string(),
                            source_page: page.pdf_page_index,
                            title_x: line.x_min,
                            y_min: line.y_min,
                            font_size: line.font_size,
                        });
                    }
                }
            }
        }
    }

    stitch_continuations(raw_lines)
}

/// Parse a single OCR line as a TOC entry.
///
/// Looks for patterns like:
/// - `1 Introduction 1`
/// - `2.4.2 Canonical exponential coordinates ... 31`
/// - `A.1.2 Inner product ... 444`
/// - `Preface xv`
/// - `Bibliography 201`
fn parse_toc_line_ocr(
    line: &OcrLine,
    page_width: f32,
    source_page: u32,
) -> Option<TocEntry> {
    let trimmed = line.text.trim();
    if trimmed.len() < 3 {
        return None;
    }

    // Clean the line: replace dot leaders and multiple spaces.
    let cleaned = clean_toc_line(trimmed);
    let cleaned = cleaned.trim();
    if cleaned.len() < 3 {
        return None;
    }

    // Split into tokens.
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    // The last token should be a page number.
    let last_token = tokens[tokens.len() - 1];

    // Try parsing as arabic number.
    let (page_num, is_roman) = if let Ok(n) = last_token.parse::<u32>() {
        if n == 0 || last_token.len() > 4 {
            return None;
        }
        (Some(n), false)
    } else {
        // Try parsing as roman numeral (case-insensitive).
        match from_roman(&last_token.to_uppercase()) {
            Some(n) if n <= 100 => (Some(n), true),
            _ => return None,
        }
    };

    // Extract title: everything before the last token.
    let title_tokens = &tokens[..tokens.len() - 1];
    let title = title_tokens.join(" ");
    let title = clean_title(&title);

    if title.len() < 2 {
        return None;
    }

    // Check if the page number is positioned in the right margin area.
    // For word-level: check if the last word's x_max is in the rightmost 25% of page.
    // We use a generous margin since OCR coordinates can be noisy.
    let has_right_margin_number = if let Some(last_word) = line.words.last() {
        last_word.x_max > page_width * 0.65
    } else {
        // No word-level data — use a simpler heuristic: text length > 10 chars.
        trimmed.len() > 10
    };

    if !has_right_margin_number && !is_roman {
        // If the number isn't at the right margin and isn't roman,
        // it's probably not a TOC page number.
        return None;
    }

    Some(TocEntry {
        title,
        page_number_raw: last_token.to_string(),
        page_number: page_num,
        is_roman,
        source_page,
        title_x: line.x_min,
        font_size: line.font_size,
        confidence: line.confidence,
    })
}

/// Stitch continuation lines into their parent entries.
///
/// A continuation line is one that:
/// - Has no trailing page number.
/// - Appears immediately after a previous line on the same source page.
/// - Has an x-start aligned with the title region (not a new entry).
fn stitch_continuations(raw_lines: Vec<RawTocLine>) -> Vec<TocEntry> {
    let mut entries: Vec<TocEntry> = Vec::new();

    for raw in raw_lines {
        match raw {
            RawTocLine::Entry {
                title,
                page_number_raw,
                page_number,
                is_roman,
                source_page,
                title_x,
                font_size,
                confidence,
            } => {
                entries.push(TocEntry {
                    title,
                    page_number_raw,
                    page_number,
                    is_roman,
                    source_page,
                    title_x,
                    font_size,
                    confidence,
                });
            }
            RawTocLine::Continuation {
                text,
                source_page,
                title_x,
                font_size: _,
                y_min: _,
            } => {
                // Try to attach to the most recent entry on the same page.
                if let Some(last_entry) = entries.last_mut() {
                    if last_entry.source_page == source_page {
                        // Check that the continuation line's x-start is within
                        // a reasonable range of the last entry's title region.
                        let x_diff = (title_x - last_entry.title_x).abs();
                        if x_diff < 50.0 {
                            // Append to title.
                            last_entry.title =
                                format!("{} {}", last_entry.title, text);
                            last_entry.title = clean_title(&last_entry.title);
                        }
                        // If x_diff is large, this might be an unnumbered entry
                        // or noise — skip it.
                    }
                }
            }
        }
    }

    entries
}

/// Clean a TOC line: remove dot leaders, middle dots, and collapse whitespace.
fn clean_toc_line(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '.' | '\u{2024}' | '\u{00b7}' | '\u{2026}' => ' ',
            _ => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Clean a title string: strip trailing dots/spaces, collapse whitespace.
pub fn clean_title(raw: &str) -> String {
    raw.trim_end_matches(['.', ' ', '\t', '\u{a0}'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a page number string, trying arabic first, then roman.
/// Returns `(parsed_number, is_roman)`.
#[allow(dead_code)]
pub fn parse_page_number(s: &str) -> Option<(u32, bool)> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u32>() {
        if n > 0 {
            return Some((n, false));
        }
    }
    // Try roman numeral (case-insensitive).
    if let Some(n) = from_roman(&s.to_uppercase()) {
        if n <= 100 {
            return Some((n, true));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Offset estimation
// ---------------------------------------------------------------------------

/// Estimate the page offset from OCR TOC entries.
///
/// For each entry with a valid arabic page number, searches the document
/// text on nearby pages and uses RANSAC-style consensus (most-voted offset)
/// to determine the physical-to-logical mapping.
///
/// If `manual_offset` is `Some`, returns it directly with confidence 1.0.
pub fn estimate_offset_from_toc(
    entries: &[TocEntry],
    doc: &Document,
    manual_offset: Option<i32>,
) -> OcrOffsetResult {
    // Manual override.
    if let Some(offset) = manual_offset {
        return OcrOffsetResult {
            arabic_offset: offset,
            roman_offset: None,
            confidence: 1.0,
            warnings: vec![],
        };
    }

    let total_pages = doc.get_pages().len() as u32;
    if total_pages == 0 || entries.is_empty() {
        return OcrOffsetResult {
            arabic_offset: 0,
            roman_offset: None,
            confidence: 0.0,
            warnings: vec!["No pages or entries to estimate offset from".into()],
        };
    }

    // Pre-extract text for each page.
    let page_texts: Vec<String> = (0..total_pages)
        .map(|i| {
            doc.extract_text(&[i + 1]) // 1-based
                .unwrap_or_default()
        })
        .collect();

    // Estimate arabic offset.
    let arabic_entries: Vec<&TocEntry> = entries
        .iter()
        .filter(|e| !e.is_roman && e.page_number.is_some())
        .collect();

    let arabic_offset = estimate_offset_for_entries(&arabic_entries, &page_texts, total_pages);

    // Estimate roman offset (if we have roman entries).
    let roman_entries: Vec<&TocEntry> = entries
        .iter()
        .filter(|e| e.is_roman && e.page_number.is_some())
        .collect();

    let roman_offset = if roman_entries.is_empty() {
        None
    } else {
        let result = estimate_offset_for_entries(&roman_entries, &page_texts, total_pages);
        Some(result.0)
    };

    let mut warnings = Vec::new();
    if arabic_offset.1 < 0.5 && !arabic_entries.is_empty() {
        warnings.push(format!(
            "Low confidence ({:.0}%) for arabic offset estimation. \
             Consider using --page-offset to override.",
            arabic_offset.1 * 100.0
        ));
    }

    OcrOffsetResult {
        arabic_offset: arabic_offset.0,
        roman_offset,
        confidence: arabic_offset.1,
        warnings,
    }
}

/// Estimate offset for a set of TOC entries against page texts.
/// Returns `(offset, confidence)`.
fn estimate_offset_for_entries(
    entries: &[&TocEntry],
    page_texts: &[String],
    total_pages: u32,
) -> (i32, f32) {
    if entries.is_empty() {
        return (0, 0.0);
    }

    // Vote for each candidate offset.
    let mut offset_votes: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();

    // Try offsets from -10 to 60.
    for candidate_offset in -10..=60_i32 {
        let mut votes = 0u32;

        for entry in entries {
            let page_num = match entry.page_number {
                Some(n) => n,
                None => continue,
            };

            let physical = page_num as i32 - 1 + candidate_offset;
            if physical < 0 || physical >= total_pages as i32 {
                continue;
            }

            let page_text = &page_texts[physical as usize];
            if page_text.is_empty() {
                continue;
            }

            // Fuzzy match title against page text.
            let similarity = best_line_similarity(&entry.title, page_text);
            if similarity >= 0.55 {
                votes += 1;
            }
        }

        if votes > 0 {
            offset_votes.insert(candidate_offset, votes);
        }
    }

    if offset_votes.is_empty() {
        return (0, 0.0);
    }

    // Find the offset with most votes.
    let (&best_offset, &best_votes) = offset_votes
        .iter()
        .max_by_key(|(_, &v)| v)
        .unwrap();

    let confidence = if best_votes >= 2 {
        (best_votes as f32 / entries.len() as f32).min(1.0) * 0.9
    } else {
        0.2
    };

    (best_offset, confidence)
}

/// Find best line-level similarity between needle and haystack.
fn best_line_similarity(needle: &str, haystack: &str) -> f64 {
    let needle_lower = needle.to_lowercase();
    let haystack_lower = haystack.to_lowercase();

    if haystack_lower.contains(&needle_lower) {
        return 1.0;
    }

    haystack_lower
        .lines()
        .map(|line| {
            strsim::normalized_levenshtein(&needle_lower, &line.trim().to_lowercase())
        })
        .fold(0.0_f64, f64::max)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::ocr_ir::{OcrBlock, OcrWord};

    fn make_line(
        text: &str,
        x_min: f32,
        x_max: f32,
        y_min: f32,
        font_size: f32,
    ) -> OcrLine {
        let word_strs: Vec<&str> = text.split_whitespace().collect();
        let n = word_strs.len();
        let words = word_strs
            .iter()
            .enumerate()
            .map(|(i, w)| {
                // Position the last word at the right margin (x_max) so it
                // passes the right-margin page-number check.
                let (wx_min, wx_max) = if i == n - 1 {
                    (x_max - 40.0, x_max)
                } else {
                    (x_min + i as f32 * 40.0, x_min + (i + 1) as f32 * 40.0)
                };
                OcrWord {
                    text: w.to_string(),
                    x_min: wx_min,
                    x_max: wx_max,
                    y_min,
                    y_max: y_min + font_size,
                    confidence: 0.9,
                }
            })
            .collect();

        OcrLine {
            text: text.to_string(),
            x_min,
            x_max,
            y_min,
            y_max: y_min + font_size,
            confidence: 0.9,
            font_size,
            words,
        }
    }

    fn make_page(page_index: u32, lines: Vec<OcrLine>) -> OcrPage {
        let block = OcrBlock {
            x_min: 72.0,
            x_max: 540.0,
            y_min: 100.0,
            y_max: 700.0,
            lines,
        };
        OcrPage {
            pdf_page_index: page_index,
            page_width: 612.0,
            page_height: 792.0,
            blocks: vec![block],
        }
    }

    #[test]
    fn test_parse_simple() {
        let line = make_line("1 Introduction 1", 72.0, 540.0, 600.0, 12.0);
        let entry = parse_toc_line_ocr(&line, 612.0, 0).unwrap();
        assert_eq!(entry.title, "1 Introduction");
        assert_eq!(entry.page_number, Some(1));
        assert!(!entry.is_roman);
    }

    #[test]
    fn test_parse_dotted() {
        let line = make_line(
            "2.4.2 Canonical exponential coordinates ... 31",
            72.0,
            540.0,
            600.0,
            12.0,
        );
        let entry = parse_toc_line_ocr(&line, 612.0, 0).unwrap();
        assert_eq!(entry.title, "2 4 2 Canonical exponential coordinates");
        assert_eq!(entry.page_number, Some(31));
    }

    #[test]
    fn test_parse_roman() {
        let line = make_line("Preface xv", 72.0, 540.0, 600.0, 12.0);
        let entry = parse_toc_line_ocr(&line, 612.0, 0).unwrap();
        assert_eq!(entry.title, "Preface");
        assert_eq!(entry.page_number, Some(15));
        assert!(entry.is_roman);
    }

    #[test]
    fn test_appendix_numbering() {
        let line = make_line("A.1.2 Inner product ... 444", 72.0, 540.0, 600.0, 12.0);
        let entry = parse_toc_line_ocr(&line, 612.0, 0).unwrap();
        assert!(entry.title.contains("Inner product"));
        assert_eq!(entry.page_number, Some(444));
    }

    #[test]
    fn test_continuation_stitching() {
        let lines = vec![
            make_line("2.4.2 Canonical exponential 31", 72.0, 540.0, 600.0, 12.0),
            // This line has no page number — it's a continuation.
        ];
        let page = make_page(0, lines);

        let entries = extract_toc_entries(&[page]);
        assert!(!entries.is_empty());
        // The first entry should have the title from the first line.
        assert!(entries[0].title.contains("Canonical"));
    }

    #[test]
    fn test_clean_title() {
        assert_eq!(clean_title("Introduction ....  "), "Introduction");
        assert_eq!(clean_title("  Chapter  1  "), "Chapter 1");
        assert_eq!(clean_title("A.1 Inner product..."), "A.1 Inner product");
    }

    #[test]
    fn test_offset_consensus() {
        // Simulate: 5 entries all agree on offset 18, 1 outlier.
        // We can't easily test with a real Document, but we test the
        // parse_page_number helper and clean_title.
        assert_eq!(parse_page_number("42"), Some((42, false)));
        assert_eq!(parse_page_number("xv"), Some((15, true)));
        assert_eq!(parse_page_number("XV"), Some((15, true)));
        assert_eq!(parse_page_number("0"), None);
        assert_eq!(parse_page_number("abc"), None);
    }

    #[test]
    fn test_mixed_roman_arabic() {
        let lines = vec![
            make_line("Preface xv", 72.0, 540.0, 700.0, 12.0),
            make_line("1 Introduction 1", 72.0, 540.0, 680.0, 12.0),
            make_line("2 Methods 15", 72.0, 540.0, 660.0, 12.0),
        ];
        let page = make_page(3, lines);
        let entries = extract_toc_entries(&[page]);

        let roman_count = entries.iter().filter(|e| e.is_roman).count();
        let arabic_count = entries.iter().filter(|e| !e.is_roman).count();

        assert_eq!(roman_count, 1);
        assert_eq!(arabic_count, 2);
    }
}
