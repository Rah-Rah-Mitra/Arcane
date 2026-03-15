//! Position-aware text extraction and structural anchor detection.
//!
//! Tracks the PDF text-matrix state machine (`BT`, `Tm`, `Td`, `TD`, `T*`,
//! `Tf`) to extract text runs with `(x, y)` coordinates.  This enables:
//!
//! - Detecting headings by vertical position + font size
//! - Finding TOC pages (pages dense with "title ... page_number" patterns)
//! - Extracting page numbers from header/footer regions

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use super::clustering::{assign_roles, cluster_font_sizes, FontCluster, FontRole};
use super::heuristics::{build_font_histogram, dominant_body_size, get_page_content_bytes, obj_as_f32, pdf_obj_to_string};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A text run with position information extracted from a content stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionedText {
    /// 0-based physical page index.
    pub page_index: u32,
    /// X coordinate (points from left edge).
    pub x: f32,
    /// Y coordinate (points from bottom edge).
    pub y: f32,
    /// Font size from the `Tf` operator.
    pub font_size: f32,
    /// Font resource key (e.g. "F1").
    pub font_key: String,
    /// The extracted text.
    pub text: String,
}

/// A structural anchor detected from layout analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutAnchor {
    /// 0-based physical page index.
    pub page_index: u32,
    /// Kind of structural element.
    pub kind: AnchorKind,
    /// The text content.
    pub text: String,
    /// Font size.
    pub font_size: f32,
    /// Y coordinate on the page.
    pub y: f32,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f32,
}

/// Kind of structural anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    /// Large-font text likely a chapter heading.
    ChapterHeading,
    /// Medium-font text likely a section heading.
    SectionHeading,
    /// Line matching "Chapter N" or numbered pattern.
    NumberedHeading,
    /// TOC entry: "Title ....... page_number" pattern.
    TocEntry,
    /// Page number in header/footer region.
    PageNumber,
}

/// Complete layout analysis result for a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutResult {
    /// Path to the analysed file.
    pub path: String,
    /// Total number of physical pages.
    pub total_pages: u32,
    /// Dominant body font size.
    pub body_font_size: f32,
    /// Detected structural anchors.
    pub anchors: Vec<LayoutAnchor>,
    /// Font-size clusters with assigned roles.
    pub font_clusters: Vec<FontCluster>,
}

// ---------------------------------------------------------------------------
// Text-matrix state machine
// ---------------------------------------------------------------------------

/// Extract positioned text runs from a single page.
///
/// Tracks the text matrix through `BT`/`ET`, `Tm`, `Td`, `TD`, `T*`, and
/// `Tf` operators.  Each text-showing operator (`Tj`, `TJ`, `'`, `"`) emits
/// a `PositionedText` entry with the current (x, y) from the text matrix.
pub fn extract_positioned_text(
    doc: &Document,
    page_oid: ObjectId,
    page_index: u32,
) -> Vec<PositionedText> {
    let bytes = match get_page_content_bytes(doc, page_oid) {
        Some(b) if !b.is_empty() => b,
        _ => return vec![],
    };
    let content = match lopdf::content::Content::decode(&bytes) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut results = Vec::new();

    // Text-matrix state: [a, b, c, d, e, f] where (e, f) = (x, y).
    let mut tm_x: f32 = 0.0;
    let mut tm_y: f32 = 0.0;
    let mut current_size: f32 = 12.0;
    let mut current_font = String::new();
    let mut leading: f32 = 0.0;

    for op in &content.operations {
        match op.operator.as_str() {
            // Begin text object — reset text matrix.
            "BT" => {
                tm_x = 0.0;
                tm_y = 0.0;
            }
            // Set text matrix: Tm a b c d e f
            "Tm" => {
                if op.operands.len() >= 6 {
                    tm_x = obj_as_f32(&op.operands[4]).unwrap_or(tm_x);
                    tm_y = obj_as_f32(&op.operands[5]).unwrap_or(tm_y);
                }
            }
            // Translate text matrix: Td tx ty
            "Td" => {
                if op.operands.len() >= 2 {
                    let tx = obj_as_f32(&op.operands[0]).unwrap_or(0.0);
                    let ty = obj_as_f32(&op.operands[1]).unwrap_or(0.0);
                    tm_x += tx;
                    tm_y += ty;
                }
            }
            // Translate + set leading: TD tx ty (equivalent to -ty TL; tx ty Td)
            "TD" => {
                if op.operands.len() >= 2 {
                    let tx = obj_as_f32(&op.operands[0]).unwrap_or(0.0);
                    let ty = obj_as_f32(&op.operands[1]).unwrap_or(0.0);
                    leading = -ty;
                    tm_x += tx;
                    tm_y += ty;
                }
            }
            // Set leading: TL leading
            "TL" => {
                if let Some(l) = op.operands.first().and_then(|o| obj_as_f32(o)) {
                    leading = l;
                }
            }
            // Move to start of next line: T* (equivalent to 0 -TL Td)
            "T*" => {
                tm_y -= leading;
            }
            // Set font: Tf name size
            "Tf" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    current_font = String::from_utf8_lossy(name).to_string();
                }
                if let Some(size_obj) = op.operands.get(1) {
                    if let Some(size) = obj_as_f32(size_obj) {
                        current_size = size;
                    }
                }
            }
            // Show text: Tj (string)
            "Tj" => {
                if let Some(text_obj) = op.operands.first() {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_size,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            // Show text with individual glyph positioning: TJ [array]
            "TJ" => {
                if let Some(Object::Array(arr)) = op.operands.first() {
                    let text: String = arr
                        .iter()
                        .filter_map(|o| match o {
                            Object::String(_, _) => Some(pdf_obj_to_string(o)),
                            _ => None,
                        })
                        .collect();
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_size,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            // Move to next line and show text: ' (string)
            "'" => {
                tm_y -= leading;
                if let Some(text_obj) = op.operands.first() {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_size,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            // Set word/char spacing, move to next line, show text: " aw ac (string)
            "\"" => {
                tm_y -= leading;
                if let Some(text_obj) = op.operands.get(2) {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_size,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    results
}

/// Extract positioned text from all pages of the document.
pub fn extract_all_positioned(doc: &Document) -> Vec<PositionedText> {
    let pages = doc.get_pages();
    let mut all = Vec::new();
    for (&page_num, &page_oid) in &pages {
        let page_index = page_num.saturating_sub(1);
        all.extend(extract_positioned_text(doc, page_oid, page_index));
    }
    all
}

// ---------------------------------------------------------------------------
// Anchor detection
// ---------------------------------------------------------------------------

/// Detect structural anchors from positioned text.
pub fn detect_anchors(
    positioned: &[PositionedText],
    body_size: f32,
    clusters: &[FontCluster],
) -> Vec<LayoutAnchor> {
    let mut anchors = Vec::new();

    // Build lookup: font_size → role.
    let role_for = |size: f32| -> Option<FontRole> {
        clusters
            .iter()
            .min_by_key(|c| ((c.centroid - size).abs() * 100.0) as u32)
            .map(|c| c.role)
    };

    for pt in positioned {
        let role = role_for(pt.font_size);
        let trimmed = pt.text.trim();

        // Chapter heading: Heading1 role.
        if role == Some(FontRole::Heading1) && trimmed.len() > 1 {
            anchors.push(LayoutAnchor {
                page_index: pt.page_index,
                kind: AnchorKind::ChapterHeading,
                text: trimmed.to_string(),
                font_size: pt.font_size,
                y: pt.y,
                confidence: 0.9,
            });
            continue;
        }

        // Section heading: Heading2 role.
        if role == Some(FontRole::Heading2) && trimmed.len() > 1 {
            anchors.push(LayoutAnchor {
                page_index: pt.page_index,
                kind: AnchorKind::SectionHeading,
                text: trimmed.to_string(),
                font_size: pt.font_size,
                y: pt.y,
                confidence: 0.8,
            });
            continue;
        }

        // Numbered heading pattern: "Chapter 1", "1.2 Foo", "Part III", etc.
        if pt.font_size >= body_size * 1.1 && is_numbered_heading(trimmed) {
            anchors.push(LayoutAnchor {
                page_index: pt.page_index,
                kind: AnchorKind::NumberedHeading,
                text: trimmed.to_string(),
                font_size: pt.font_size,
                y: pt.y,
                confidence: 0.85,
            });
            continue;
        }

        // TOC entry pattern: "Title ... number" or "Title\tnumber".
        if is_toc_entry(trimmed) {
            anchors.push(LayoutAnchor {
                page_index: pt.page_index,
                kind: AnchorKind::TocEntry,
                text: trimmed.to_string(),
                font_size: pt.font_size,
                y: pt.y,
                confidence: 0.7,
            });
            continue;
        }
    }

    anchors
}

/// Detect pages that are likely part of a Table of Contents.
///
/// A TOC page typically has many lines matching the "title ... page_number"
/// pattern.  Returns 0-based page indices.
pub fn detect_toc_pages(positioned: &[PositionedText]) -> Vec<u32> {
    use std::collections::BTreeMap;

    let mut toc_counts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut total_counts: BTreeMap<u32, u32> = BTreeMap::new();

    for pt in positioned {
        *total_counts.entry(pt.page_index).or_insert(0) += 1;
        if is_toc_entry(pt.text.trim()) {
            *toc_counts.entry(pt.page_index).or_insert(0) += 1;
        }
    }

    // A page is a TOC page if ≥ 3 TOC entries AND ≥ 30% of text runs are TOC entries.
    toc_counts
        .into_iter()
        .filter(|&(page, count)| {
            let total = total_counts.get(&page).copied().unwrap_or(1);
            count >= 3 && (count as f32 / total as f32) >= 0.3
        })
        .map(|(page, _)| page)
        .collect()
}

/// Detect page numbers in header/footer regions.
///
/// Looks for isolated small numbers (1-4 digits) positioned in the top or
/// bottom 10% of the page height.  Returns `(page_index, detected_number)`.
pub fn detect_page_numbers(positioned: &[PositionedText], page_height: f32) -> Vec<(u32, u32)> {
    let margin = page_height * 0.10;
    let mut results = Vec::new();

    for pt in positioned {
        // Must be in header (top 10%) or footer (bottom 10%) region.
        let in_footer = pt.y < margin;
        let in_header = pt.y > (page_height - margin);

        if !in_footer && !in_header {
            continue;
        }

        let trimmed = pt.text.trim();
        // Must be a small number (1-4 digits).
        if trimmed.len() >= 1 && trimmed.len() <= 4 && trimmed.chars().all(|c| c.is_ascii_digit())
        {
            if let Ok(num) = trimmed.parse::<u32>() {
                if num > 0 {
                    results.push((pt.page_index, num));
                }
            }
        }
    }

    results
}

/// Full layout analysis pipeline for a document.
pub fn analyze_layout(doc: &Document, path: &str) -> LayoutResult {
    let pages = doc.get_pages();
    let total_pages = pages.len() as u32;

    // Extract positioned text.
    let positioned = extract_all_positioned(doc);

    // Determine body size and clusters.
    let histogram = build_font_histogram(doc).unwrap_or_default();
    let body_font_size = dominant_body_size(&histogram).unwrap_or(12.0);

    let raw_clusters = cluster_font_sizes(&histogram, 6);
    let font_clusters = assign_roles(&raw_clusters);

    // Detect anchors.
    let anchors = detect_anchors(&positioned, body_font_size, &font_clusters);

    LayoutResult {
        path: path.to_string(),
        total_pages,
        body_font_size,
        anchors,
        font_clusters,
    }
}

// ---------------------------------------------------------------------------
// Pattern helpers
// ---------------------------------------------------------------------------

/// Check if text matches a numbered heading pattern.
fn is_numbered_heading(text: &str) -> bool {
    let lower = text.to_lowercase();

    // "Chapter 1", "Part II", "Section 3.1"
    if lower.starts_with("chapter ")
        || lower.starts_with("part ")
        || lower.starts_with("section ")
        || lower.starts_with("appendix ")
    {
        return true;
    }

    // "1 Introduction", "1.2 Methods", "A.1 Appendix"
    let first_word = text.split_whitespace().next().unwrap_or("");
    if !first_word.is_empty() && text.split_whitespace().count() > 1 {
        // Check if first word is a number pattern: "1", "1.2", "1.2.3", "A.1"
        let is_number_pattern = first_word
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_uppercase());
        if is_number_pattern && first_word.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
    }

    false
}

/// Check if text matches a TOC entry pattern ("Title ... number" or "Title\tnumber").
fn is_toc_entry(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 3 {
        return false;
    }

    // Look for "text" followed by dots/spaces then a number at the end.
    // Pattern: any text, then 2+ dots or many spaces, then 1-4 digit number.
    if let Some(last_word) = trimmed.split_whitespace().last() {
        if last_word.len() <= 4
            && last_word.chars().all(|c| c.is_ascii_digit())
            && last_word.parse::<u32>().is_ok()
        {
            // Check for leader dots or sufficient whitespace before the number.
            let prefix = trimmed.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
            if prefix.contains("..") || prefix.contains("  ") || prefix.contains('\t') {
                return true;
            }
            // Also match if the text part is long enough relative to the number
            // (suggests it's a title followed by a page reference).
            let text_part = trimmed[..trimmed.len() - last_word.len()].trim();
            if text_part.len() >= 5 {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_heading_patterns() {
        assert!(is_numbered_heading("Chapter 1"));
        assert!(is_numbered_heading("Chapter 10 Introduction"));
        assert!(is_numbered_heading("Part III"));
        assert!(is_numbered_heading("Section 3.1 Methods"));
        assert!(is_numbered_heading("1 Introduction"));
        assert!(is_numbered_heading("1.2 Related Work"));
        assert!(is_numbered_heading("A.1 Appendix Details"));
        assert!(!is_numbered_heading("Introduction"));
        assert!(!is_numbered_heading("42")); // just a number, no title
    }

    #[test]
    fn toc_entry_patterns() {
        assert!(is_toc_entry("Chapter 1 .... 15"));
        assert!(is_toc_entry("Introduction ............... 1"));
        assert!(is_toc_entry("Methods   42"));
        assert!(is_toc_entry("Related Work\t10"));
        assert!(is_toc_entry("Background and Motivation 100"));
        assert!(!is_toc_entry("42")); // too short
        assert!(!is_toc_entry("ab")); // too short
        assert!(!is_toc_entry("")); // empty
    }

    #[test]
    fn detect_page_numbers_footer() {
        let positioned = vec![
            PositionedText {
                page_index: 0,
                x: 300.0,
                y: 30.0, // in footer region (< 79.2 for 792pt page)
                font_size: 10.0,
                font_key: "F1".into(),
                text: "42".into(),
            },
            PositionedText {
                page_index: 0,
                x: 100.0,
                y: 400.0, // middle of page
                font_size: 12.0,
                font_key: "F1".into(),
                text: "Some body text".into(),
            },
            PositionedText {
                page_index: 1,
                x: 300.0,
                y: 760.0, // in header region (> 712.8 for 792pt page)
                font_size: 10.0,
                font_key: "F1".into(),
                text: "43".into(),
            },
        ];

        let nums = detect_page_numbers(&positioned, 792.0);
        assert_eq!(nums.len(), 2);
        assert_eq!(nums[0], (0, 42));
        assert_eq!(nums[1], (1, 43));
    }

    #[test]
    fn text_matrix_tracking() {
        // Build a PDF with known text matrix operations.
        use lopdf::{Dictionary, Stream};

        let content = b"BT /F1 14 Tf 72 700 Td (Chapter 1) Tj ET";
        let mut doc = Document::with_version("1.7");

        let stream = Stream::new(Dictionary::new(), content.to_vec());
        let stream_id = doc.add_object(Object::Stream(stream));

        let mut page_dict = Dictionary::new();
        page_dict.set("Type", Object::Name(b"Page".to_vec()));
        page_dict.set("Contents", Object::Reference(stream_id));
        page_dict.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        );
        let page_id = doc.add_object(Object::Dictionary(page_dict));

        let mut pages_dict = Dictionary::new();
        pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
        pages_dict.set("Count", Object::Integer(1));
        pages_dict.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        let pages_id = doc.add_object(Object::Dictionary(pages_dict));

        if let Ok(Object::Dictionary(ref mut d)) = doc.get_object_mut(page_id) {
            d.set("Parent", Object::Reference(pages_id));
        }

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        let positioned = extract_positioned_text(&doc, page_id, 0);
        assert_eq!(positioned.len(), 1);
        assert_eq!(positioned[0].text, "Chapter 1");
        assert!((positioned[0].x - 72.0).abs() < 0.1);
        assert!((positioned[0].y - 700.0).abs() < 0.1);
        assert!((positioned[0].font_size - 14.0).abs() < 0.1);
    }

    #[test]
    fn detect_toc_pages_finds_dense_pages() {
        let mut positioned = Vec::new();
        // Page 2 has many TOC entries.
        for i in 0..8 {
            positioned.push(PositionedText {
                page_index: 2,
                x: 72.0,
                y: 700.0 - (i as f32 * 20.0),
                font_size: 12.0,
                font_key: "F1".into(),
                text: format!("Chapter {} .... {}", i + 1, (i + 1) * 10),
            });
        }
        // Page 5 has no TOC entries.
        positioned.push(PositionedText {
            page_index: 5,
            x: 72.0,
            y: 700.0,
            font_size: 12.0,
            font_key: "F1".into(),
            text: "Just some body text here".into(),
        });

        let toc_pages = detect_toc_pages(&positioned);
        assert!(toc_pages.contains(&2));
        assert!(!toc_pages.contains(&5));
    }
}
