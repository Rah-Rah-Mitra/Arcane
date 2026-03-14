//! Heuristic outline recovery for PDFs that lack `/Outlines` metadata.
//!
//! # Strategy
//!
//! 1. **Font-size histogram** — scan every page's content stream and build a
//!    frequency map of `(font_size_×10_rounded → char_count)`.  The most
//!    frequent size is "body text".
//!
//! 2. **Heading extraction** — text rendered at `≥ body_size × min_ratio` is
//!    a heading candidate.  Adjacent runs on the same page / same size are
//!    merged into one `HeadingCandidate`.
//!
//! 3. **Outline injection** — given a `BTreeMap<physical_page, title>`,
//!    construct a proper PDF `/Outlines` tree and attach it to the catalog.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A heading found by font-size analysis.
#[derive(Debug, Clone)]
pub struct HeadingCandidate {
    /// 0-based physical page index.
    pub page_index: u32,
    /// Nominal font size as set by the `Tf` operator.
    pub font_size: f32,
    /// Accumulated text of the heading on that page.
    pub text: String,
}

// ---------------------------------------------------------------------------
// Phase 1: font-size histogram
// ---------------------------------------------------------------------------

/// Build a histogram of `(font_size_×10_rounded → total_char_count)` over
/// the entire document by parsing every page's content stream.
///
/// Returns `None` if no text operators were found (e.g. scanned image PDFs).
pub fn build_font_histogram(doc: &Document) -> Option<BTreeMap<u16, u64>> {
    let mut histogram: BTreeMap<u16, u64> = BTreeMap::new();
    let pages = doc.get_pages(); // BTreeMap<1-based, ObjectId>

    for page_oid in pages.values() {
        let bytes = match get_page_content_bytes(doc, *page_oid) {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };

        let content = match lopdf::content::Content::decode(&bytes) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut current_key: u16 = 120; // default 12pt × 10

        for op in &content.operations {
            match op.operator.as_str() {
                "Tf" => {
                    if let Some(size_obj) = op.operands.get(1) {
                        if let Some(size) = obj_as_f32(size_obj) {
                            current_key = float_to_key(size);
                        }
                    }
                }
                "Tj" => {
                    if let Some(text_obj) = op.operands.first() {
                        let len = pdf_obj_text_len(text_obj);
                        *histogram.entry(current_key).or_insert(0) += len;
                    }
                }
                "TJ" => {
                    if let Some(Object::Array(arr)) = op.operands.first() {
                        let len: u64 = arr
                            .iter()
                            .filter_map(|o| match o {
                                Object::String(_, _) => Some(pdf_obj_text_len(o)),
                                _ => None,
                            })
                            .sum();
                        *histogram.entry(current_key).or_insert(0) += len;
                    }
                }
                "'" | "\"" => {
                    // ' shows text after newline; " also sets word/char spacing
                    let text_idx = if op.operator == "\"" { 2 } else { 0 };
                    if let Some(text_obj) = op.operands.get(text_idx) {
                        let len = pdf_obj_text_len(text_obj);
                        *histogram.entry(current_key).or_insert(0) += len;
                    }
                }
                _ => {}
            }
        }
    }

    if histogram.is_empty() {
        None
    } else {
        Some(histogram)
    }
}

/// Return the histogram key (font_size × 10, rounded) for a body-text size.
pub fn dominant_body_size(histogram: &BTreeMap<u16, u64>) -> Option<f32> {
    histogram
        .iter()
        .max_by_key(|(_, &count)| count)
        .map(|(&key, _)| key as f32 / 10.0)
}

// ---------------------------------------------------------------------------
// Phase 2: heading extraction
// ---------------------------------------------------------------------------

/// Extract heading candidates from the document using font-size heuristics.
///
/// `min_ratio` — font sizes ≥ `body_size × min_ratio` are considered headings.
/// `max_depth` — generate candidates for up to this many heading levels
///   (level 1 = largest; level 2 = slightly smaller, etc.).
pub fn extract_headings(doc: &Document, min_ratio: f32, max_depth: u32) -> Vec<HeadingCandidate> {
    let histogram = match build_font_histogram(doc) {
        Some(h) => h,
        None => return vec![],
    };
    let body_size = match dominant_body_size(&histogram) {
        Some(s) => s,
        None => return vec![],
    };

    tracing::debug!(
        body_size,
        threshold = body_size * min_ratio,
        "Heading extraction thresholds"
    );

    // Build size thresholds for each depth level.
    // depth-1 threshold = body × min_ratio
    // depth-2 threshold = body × (min_ratio × 0.85)   (a bit smaller)
    // depth-N threshold = body × (min_ratio × 0.85^(N-1))
    let thresholds: Vec<f32> = (1..=max_depth)
        .map(|d| body_size * min_ratio * 0.85_f32.powi(d as i32 - 1))
        .collect();

    // Minimum threshold across all depths — anything below this is body text.
    let min_threshold = thresholds.last().copied().unwrap_or(body_size * min_ratio);

    let pages = doc.get_pages();
    let mut candidates: Vec<HeadingCandidate> = Vec::new();

    // Accumulator for the current heading being built.
    let mut acc_page: Option<u32> = None;
    let mut acc_size: f32 = 0.0;
    let mut acc_text = String::new();

    let flush = |acc_page: &mut Option<u32>,
                 acc_size: &mut f32,
                 acc_text: &mut String,
                 candidates: &mut Vec<HeadingCandidate>| {
        if let Some(page) = acc_page.take() {
            let text = acc_text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                candidates.push(HeadingCandidate {
                    page_index: page,
                    font_size: *acc_size,
                    text,
                });
            }
            *acc_text = String::new();
            *acc_size = 0.0;
        }
    };

    for (page_num, page_oid) in &pages {
        let physical_page = page_num.saturating_sub(1); // 0-based

        let bytes = match get_page_content_bytes(doc, *page_oid) {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let content = match lopdf::content::Content::decode(&bytes) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut current_size: f32 = 12.0;

        for op in &content.operations {
            match op.operator.as_str() {
                "Tf" => {
                    if let Some(size_obj) = op.operands.get(1) {
                        if let Some(size) = obj_as_f32(size_obj) {
                            current_size = size;
                        }
                    }
                }
                "Tj" | "'" => {
                    if current_size >= min_threshold {
                        if let Some(text_obj) = op.operands.first() {
                            let text = pdf_obj_to_string(text_obj);
                            if !text.trim().is_empty() {
                                // Flush if switching page or significantly different size.
                                if acc_page != Some(physical_page)
                                    || (acc_size - current_size).abs() > 0.5
                                {
                                    flush(
                                        &mut acc_page,
                                        &mut acc_size,
                                        &mut acc_text,
                                        &mut candidates,
                                    );
                                }
                                acc_page = Some(physical_page);
                                acc_size = current_size;
                                acc_text.push_str(&text);
                                acc_text.push(' ');
                            }
                        }
                    } else {
                        // Body-text operator — flush any pending heading.
                        flush(&mut acc_page, &mut acc_size, &mut acc_text, &mut candidates);
                    }
                }
                "TJ" => {
                    if current_size >= min_threshold {
                        if let Some(Object::Array(arr)) = op.operands.first() {
                            let text: String = arr
                                .iter()
                                .filter_map(|o| match o {
                                    Object::String(_, _) => Some(pdf_obj_to_string(o)),
                                    _ => None,
                                })
                                .collect();
                            if !text.trim().is_empty() {
                                if acc_page != Some(physical_page)
                                    || (acc_size - current_size).abs() > 0.5
                                {
                                    flush(
                                        &mut acc_page,
                                        &mut acc_size,
                                        &mut acc_text,
                                        &mut candidates,
                                    );
                                }
                                acc_page = Some(physical_page);
                                acc_size = current_size;
                                acc_text.push_str(&text);
                                acc_text.push(' ');
                            }
                        }
                    } else {
                        flush(&mut acc_page, &mut acc_size, &mut acc_text, &mut candidates);
                    }
                }
                // BT/ET reset context — flush on ET.
                "ET" => {
                    flush(&mut acc_page, &mut acc_size, &mut acc_text, &mut candidates);
                }
                _ => {}
            }
        }

        // Flush at end of page.
        flush(&mut acc_page, &mut acc_size, &mut acc_text, &mut candidates);
    }

    // Post-filter: only keep headings whose text looks like a chapter/section
    // title and has a meaningful length.
    candidates
        .into_iter()
        .filter(|c| {
            let t = c.text.trim();
            // Must be non-trivial (more than 1 character, not just a number).
            t.len() > 1
                && !t
                    .chars()
                    .all(|ch| ch.is_numeric() || ch == '.' || ch == ' ')
        })
        .collect()
}

/// Collapse `Vec<HeadingCandidate>` into a chapter map.
///
/// When multiple candidates land on the same page, the largest-font one wins
/// (main chapter heading beats a section heading on the same page).
pub fn headings_to_chapter_map(headings: &[HeadingCandidate]) -> BTreeMap<u32, String> {
    let mut map: BTreeMap<u32, HeadingCandidate> = BTreeMap::new();
    for h in headings {
        map.entry(h.page_index)
            .and_modify(|existing| {
                if h.font_size > existing.font_size {
                    *existing = h.clone();
                }
            })
            .or_insert_with(|| h.clone());
    }
    map.into_iter()
        .map(|(page, h)| (page, sanitise_heading_title(&h.text)))
        .collect()
}

// ---------------------------------------------------------------------------
// Phase 3: outline injection
// ---------------------------------------------------------------------------

/// Inject a `/Outlines` tree into `doc` using the supplied `chapters` map.
///
/// If the document already has an `/Outlines` entry it is overwritten.
/// Returns the number of outline entries written.
pub fn inject_outlines(doc: &mut Document, chapters: &BTreeMap<u32, String>) -> Result<usize> {
    if chapters.is_empty() {
        anyhow::bail!("no chapters to inject");
    }

    // Build a reverse page map: 0-based physical index → ObjectId.
    let page_oid_map: BTreeMap<u32, ObjectId> = doc
        .get_pages()
        .into_iter()
        .map(|(page_num, oid)| (page_num.saturating_sub(1), oid))
        .collect();

    // Collect (page_oid, title) for chapters that exist in the document.
    let entries: Vec<(ObjectId, String)> = chapters
        .iter()
        .filter_map(|(&page_idx, title)| {
            page_oid_map.get(&page_idx).map(|&oid| (oid, title.clone()))
        })
        .collect();

    if entries.is_empty() {
        anyhow::bail!("none of the detected chapter pages are present in the document");
    }

    let n = entries.len();

    // ── First pass: add all outline items without Next/Prev/Parent ──────────
    // We use a placeholder Reference(0,0) for Parent and patch it later.
    let placeholder = Object::Reference((0, 0));

    let ids: Vec<ObjectId> = entries
        .iter()
        .map(|(page_oid, title)| {
            let dest = Object::Array(vec![
                Object::Reference(*page_oid),
                Object::Name(b"XYZ".to_vec()),
                Object::Null,
                Object::Null,
                Object::Null,
            ]);
            let mut d = Dictionary::new();
            d.set("Title", Object::string_literal(title.as_str()));
            d.set("Parent", placeholder.clone());
            d.set("Dest", dest);
            doc.add_object(Object::Dictionary(d))
        })
        .collect();

    // ── Create the root outlines dictionary ──────────────────────────────────
    let mut root_dict = Dictionary::new();
    root_dict.set("Type", Object::Name(b"Outlines".to_vec()));
    root_dict.set("Count", Object::Integer(n as i64));
    root_dict.set("First", Object::Reference(ids[0]));
    root_dict.set("Last", Object::Reference(ids[n - 1]));
    let root_id = doc.add_object(Object::Dictionary(root_dict));

    // ── Second pass: patch Parent, Next, Prev on each item ───────────────────
    for i in 0..n {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(ids[i]) {
            d.set("Parent", Object::Reference(root_id));
            if i > 0 {
                d.set("Prev", Object::Reference(ids[i - 1]));
            }
            if i < n - 1 {
                d.set("Next", Object::Reference(ids[i + 1]));
            }
        }
    }

    // ── Attach outlines to the catalog ───────────────────────────────────────
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .context("PDF has no /Root in trailer")?
        .as_reference()
        .context("PDF /Root is not a reference")?;

    if let Ok(Object::Dictionary(catalog)) = doc.get_object_mut(catalog_id) {
        catalog.set("Outlines", Object::Reference(root_id));
    }

    Ok(n)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract decompressed content bytes from all `/Contents` streams of a page.
fn get_page_content_bytes(doc: &Document, page_oid: ObjectId) -> Option<Vec<u8>> {
    let page_obj = doc.get_object(page_oid).ok()?;
    let page_dict = page_obj.as_dict().ok()?;

    let contents = page_dict.get(b"Contents").ok()?;

    let stream_ids: Vec<ObjectId> = match contents {
        Object::Reference(r) => vec![*r],
        Object::Array(refs) => refs.iter().filter_map(|o| o.as_reference().ok()).collect(),
        _ => return None,
    };

    let mut all_bytes: Vec<u8> = Vec::new();
    for sid in stream_ids {
        let obj = doc.get_object(sid).ok()?;
        if let Ok(stream) = obj.as_stream() {
            let mut s: Stream = stream.clone();
            // Decompress FlateDecode / LZW filters if present.
            // decompress() is best-effort — it silently leaves bytes as-is on
            // unknown filters so Content::decode may still fail, which we handle.
            let _ = s.decompress();
            all_bytes.extend_from_slice(&s.content);
            all_bytes.push(b' '); // separator between streams
        }
    }

    if all_bytes.is_empty() {
        None
    } else {
        Some(all_bytes)
    }
}

/// Convert a PDF String object to a Rust `String` (best-effort UTF-8).
fn pdf_obj_to_string(obj: &Object) -> String {
    match obj {
        Object::String(bytes, _) => {
            // Handle UTF-16-BE BOM.
            if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                let u16s: Vec<u16> = bytes[2..]
                    .chunks_exact(2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect();
                String::from_utf16_lossy(&u16s)
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
        _ => String::new(),
    }
}

/// Count the number of characters in a text-show operand (for histogram).
fn pdf_obj_text_len(obj: &Object) -> u64 {
    match obj {
        Object::String(bytes, _) => bytes.len() as u64,
        _ => 0,
    }
}

/// Extract a numeric value from a lopdf `Object` that may be Real or Integer.
fn obj_as_f32(obj: &Object) -> Option<f32> {
    match obj {
        Object::Real(f) => Some(*f),
        Object::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

/// Convert a float font size to a histogram key (size × 10, rounded).
fn float_to_key(size: f32) -> u16 {
    (size * 10.0).round() as u16
}

/// Clean up a heading title for use as a chapter name.
fn sanitise_heading_title(text: &str) -> String {
    // Collapse whitespace, trim, then apply filename-safe cleanup.
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Don't run through sanitise_filename (that replaces spaces with underscores);
    // chapter names should have spaces.  Just trim.
    clean.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_to_key_rounds_correctly() {
        assert_eq!(float_to_key(12.0), 120);
        assert_eq!(float_to_key(11.95), 120); // rounds to 120
        assert_eq!(float_to_key(8.5), 85);
    }

    #[test]
    fn headings_to_chapter_map_deduplicates_by_largest_font() {
        let headings = vec![
            HeadingCandidate {
                page_index: 0,
                font_size: 14.0,
                text: "Section 1".into(),
            },
            HeadingCandidate {
                page_index: 0,
                font_size: 18.0,
                text: "Chapter 1".into(),
            },
            HeadingCandidate {
                page_index: 5,
                font_size: 18.0,
                text: "Chapter 2".into(),
            },
        ];
        let map = headings_to_chapter_map(&headings);
        assert_eq!(map.len(), 2);
        assert_eq!(map[&0], "Chapter 1"); // larger font wins
        assert_eq!(map[&5], "Chapter 2");
    }

    #[test]
    fn sanitise_heading_title_collapses_whitespace() {
        assert_eq!(
            sanitise_heading_title("  Chapter  1  Introduction  "),
            "Chapter 1 Introduction"
        );
    }

    #[test]
    fn dominant_body_size_picks_most_frequent() {
        let mut h = BTreeMap::new();
        h.insert(120u16, 10_000u64); // 12pt – most frequent → body
        h.insert(180u16, 50u64); // 18pt – heading
        h.insert(140u16, 200u64); // 14pt – subheading
        assert_eq!(dominant_body_size(&h), Some(12.0));
    }
}
