//! Position-aware text extraction and multi-heuristic structural classification.
//!
//! ## Feature-Vector Pipeline
//!
//! 1. [`extract_positioned_text`] — raw text runs with Tm-scale-corrected font sizes.
//! 2. [`build_typographic_profile`] — document-wide statistics (μ, σ, mode, p90)
//!    over the first 50 pages; replaces hardcoded multipliers with Z-score thresholds.
//! 3. [`build_text_features`] — enriches each run with bit-flag features, Z-score,
//!    case pattern, font-descriptor weight/italic data, and y-gap above.
//! 4. [`classify_features`] — multi-heuristic classification with optional Bayesian
//!    confidence boosts from TOC matching and a predicted page offset.
//!
//! The legacy [`detect_anchors`] (clustering-only) is kept for backward compatibility.

use std::collections::{BTreeMap, HashMap};

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use super::clustering::{assign_roles, cluster_font_sizes, FontCluster};
use super::heuristics::{
    build_font_histogram, get_page_content_bytes, obj_as_f32, pdf_obj_to_string,
};

// ---------------------------------------------------------------------------
// Raw extraction type
// ---------------------------------------------------------------------------

/// A text run with position information extracted from a content stream.
///
/// `font_size` is the *effective* size: `Tf_nominal × √(a²+b²)` from the
/// most recent `Tm` operator — correcting for PDFs that use a nominal 1pt
/// font scaled entirely via the text matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionedText {
    /// 0-based physical page index.
    pub page_index: u32,
    /// X coordinate (points from left edge).
    pub x: f32,
    /// Y coordinate (points from bottom edge).
    pub y: f32,
    /// Effective font size (`Tf_nominal × Tm_scale`).
    pub font_size: f32,
    /// Font resource key (e.g. `"F1"`).
    pub font_key: String,
    /// The extracted text.
    pub text: String,
}

// ---------------------------------------------------------------------------
// Feature-vector types
// ---------------------------------------------------------------------------

/// Bit-packed text feature flags (packed `u16` — no external crate required).
pub type TextFlags = u16;

/// Font weight > 600, or `BaseFont` name contains `"Bold"`/`"Heavy"`/`"Black"`/`"-Bd"`.
pub const FLAG_BOLD: TextFlags = 0x0001;
/// `ItalicAngle` from `/FontDescriptor` is < −5°.
pub const FLAG_ITALIC: TextFlags = 0x0002;
/// Every alphabetic character in the text run is uppercase.
pub const FLAG_ALL_CAPS: TextFlags = 0x0004;
/// Each word starts with an uppercase letter.
pub const FLAG_TITLE_CASE: TextFlags = 0x0008;
/// Vertical gap above ≥ 90th-percentile gap of the document (`FLAG_ISOLATED`).
pub const FLAG_ISOLATED: TextFlags = 0x0010;
/// Z-score of font size > 3.0 — high-confidence size outlier.
pub const FLAG_LARGE_FONT: TextFlags = 0x0020;
/// Z-score of font size in \[1.5, 3.0).
pub const FLAG_MED_FONT: TextFlags = 0x0040;
/// Z-score of font size < −1.5 — footnote / page-number territory.
pub const FLAG_SMALL_FONT: TextFlags = 0x0080;

/// Case distribution of a text run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CasePattern {
    /// All alphabetic characters are uppercase.
    AllCaps,
    /// Each word starts with an uppercase letter.
    TitleCase,
    /// Only the first word starts with an uppercase letter.
    SentenceCase,
    /// Inconsistent casing.
    Mixed,
    /// No alphabetic characters (numbers, punctuation, symbols).
    Numeric,
}

/// Enriched text run with feature flags, statistical scores, and spatial context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextFeature {
    /// 0-based physical page index.
    pub page_index: u32,
    /// X coordinate (points from left edge).
    pub x: f32,
    /// Y coordinate (points from bottom edge).
    pub y: f32,
    /// Effective font size.
    pub effective_font_size: f32,
    /// Z-score: `(effective_font_size − μ) / σ` using document-wide statistics.
    pub size_z: f32,
    /// Font resource key.
    pub font_key: String,
    /// The text content.
    pub text: String,
    /// Bit-packed feature flags (see `FLAG_*` constants).
    pub flags: TextFlags,
    /// Font weight from `/FontDescriptor/FontWeight` (0 = unknown, 400 = regular, 700 = bold).
    pub font_weight: u16,
    /// Italic angle from `/FontDescriptor/ItalicAngle` (0.0 = upright).
    pub italic_angle: f32,
    /// Vertical gap to the nearest text block above on this page (points).
    pub y_gap_above: f32,
    /// Case distribution of the text content.
    pub case_pattern: CasePattern,
}

impl TextFeature {
    /// Test whether a feature flag is set.
    #[inline]
    pub fn is(&self, flag: TextFlags) -> bool {
        self.flags & flag != 0
    }
}

// ---------------------------------------------------------------------------
// Statistical profile
// ---------------------------------------------------------------------------

/// Document-wide typographic statistics computed from a sample of pages.
///
/// Replaces hardcoded multipliers (e.g. `body_size × 1.3`) with data-driven
/// Z-score thresholds calibrated to the actual document.
#[derive(Debug, Clone)]
pub struct TypographicProfile {
    /// Mean effective font size across sampled text runs.
    pub size_mean: f32,
    /// Standard deviation of effective font sizes.
    pub size_stddev: f32,
    /// Mode of the font-size distribution — the body-text centroid.
    pub body_centroid: f32,
    /// 90th-percentile y-gap threshold (used for `FLAG_ISOLATED`).
    pub gap_p90: f32,
}

impl Default for TypographicProfile {
    fn default() -> Self {
        TypographicProfile {
            size_mean: 12.0,
            size_stddev: 2.0,
            body_centroid: 12.0,
            gap_p90: 28.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Structural anchor types
// ---------------------------------------------------------------------------

/// A structural anchor detected from layout analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutAnchor {
    /// 0-based physical page index.
    pub page_index: u32,
    /// Kind of structural element.
    pub kind: AnchorKind,
    /// The text content.
    pub text: String,
    /// Effective font size.
    pub font_size: f32,
    /// Y coordinate on the page.
    pub y: f32,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// Kind of structural anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    /// Large-font or bold+isolated text likely a chapter heading.
    ChapterHeading,
    /// Medium-font or bold-isolated text likely a section heading.
    SectionHeading,
    /// Line matching "Chapter N", "1.2 Foo", or similar numbered pattern.
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
    /// Body-text centroid (mode of the font-size distribution).
    pub body_font_size: f32,
    /// Detected structural anchors.
    pub anchors: Vec<LayoutAnchor>,
    /// Font-size clusters with assigned roles.
    pub font_clusters: Vec<FontCluster>,
    /// Full feature vectors (omitted from JSON when empty).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<TextFeature>,
}

// ---------------------------------------------------------------------------
// Phase 1 — Text-matrix state machine (Tm-scale-corrected extraction)
// ---------------------------------------------------------------------------

/// Extract positioned text runs from a single page.
///
/// Tracks `BT`/`ET`, `Tm`, `Td`, `TD`, `T*`, and `Tf` operators.
/// The effective font size is `Tf_nominal × √(a²+b²)` from the `Tm` matrix,
/// so PDFs that use a nominal `1pt` font scaled by the matrix are handled correctly.
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

    let mut tm_x: f32 = 0.0;
    let mut tm_y: f32 = 0.0;
    // Scale factor √(a²+b²) from the most recent `Tm` operator.
    let mut tm_scale: f32 = 1.0;
    let mut current_nominal: f32 = 12.0; // raw `Tf` size
    let mut current_font = String::new();
    let mut leading: f32 = 0.0;

    for op in &content.operations {
        match op.operator.as_str() {
            // Begin text object — reset text matrix (not font).
            "BT" => {
                tm_x = 0.0;
                tm_y = 0.0;
                tm_scale = 1.0;
            }
            // Set text matrix: Tm a b c d e f
            "Tm" => {
                if op.operands.len() >= 6 {
                    let a = obj_as_f32(&op.operands[0]).unwrap_or(1.0);
                    let b = obj_as_f32(&op.operands[1]).unwrap_or(0.0);
                    tm_scale = (a * a + b * b).sqrt().max(0.001);
                    tm_x = obj_as_f32(&op.operands[4]).unwrap_or(tm_x);
                    tm_y = obj_as_f32(&op.operands[5]).unwrap_or(tm_y);
                }
            }
            // Translate text matrix: Td tx ty
            "Td" => {
                if op.operands.len() >= 2 {
                    tm_x += obj_as_f32(&op.operands[0]).unwrap_or(0.0);
                    tm_y += obj_as_f32(&op.operands[1]).unwrap_or(0.0);
                }
            }
            // Translate + set leading: TD tx ty
            "TD" => {
                if op.operands.len() >= 2 {
                    let ty = obj_as_f32(&op.operands[1]).unwrap_or(0.0);
                    leading = -ty;
                    tm_x += obj_as_f32(&op.operands[0]).unwrap_or(0.0);
                    tm_y += ty;
                }
            }
            // Set leading: TL
            "TL" => {
                if let Some(l) = op.operands.first().and_then(obj_as_f32) {
                    leading = l;
                }
            }
            // Move to start of next line: T*
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
                        current_nominal = size;
                    }
                }
            }
            "Tj" => {
                if let Some(text_obj) = op.operands.first() {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_nominal * tm_scale,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
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
                            font_size: current_nominal * tm_scale,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            "'" => {
                tm_y -= leading;
                if let Some(text_obj) = op.operands.first() {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_nominal * tm_scale,
                            font_key: current_font.clone(),
                            text,
                        });
                    }
                }
            }
            "\"" => {
                tm_y -= leading;
                if let Some(text_obj) = op.operands.get(2) {
                    let text = pdf_obj_to_string(text_obj);
                    if !text.trim().is_empty() {
                        results.push(PositionedText {
                            page_index,
                            x: tm_x,
                            y: tm_y,
                            font_size: current_nominal * tm_scale,
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
// Phase 2 — Typographic profiler
// ---------------------------------------------------------------------------

/// Build a document-wide typographic profile from the first `max_pages` pages.
///
/// Uses integer-scaled arithmetic (`size × 10` as `u16`) in the hot path to
/// minimise floating-point overhead on large documents.
pub fn build_typographic_profile(
    positioned: &[PositionedText],
    max_pages: u32,
) -> TypographicProfile {
    // Determine the page-index cutoff.
    let cutoff_page: u32 = {
        let mut pages: Vec<u32> = positioned.iter().map(|p| p.page_index).collect();
        pages.sort_unstable();
        pages.dedup();
        pages.get(max_pages as usize).copied().unwrap_or(u32::MAX)
    };

    let sample: Vec<&PositionedText> = positioned
        .iter()
        .filter(|p| p.page_index < cutoff_page)
        .collect();

    if sample.is_empty() {
        return TypographicProfile::default();
    }

    // --- Font-size statistics ---
    // Bucket histogram: key = (size × 10).round() as u16
    let mut bucket_hist: BTreeMap<u16, u32> = BTreeMap::new();
    let mut size_sum: f64 = 0.0;
    let mut size_sum_sq: f64 = 0.0;
    let n = sample.len() as f64;

    for pt in &sample {
        let key = (pt.font_size * 10.0).round().max(0.0).min(u16::MAX as f32) as u16;
        *bucket_hist.entry(key).or_insert(0) += 1;
        let v = pt.font_size as f64;
        size_sum += v;
        size_sum_sq += v * v;
    }

    let size_mean = (size_sum / n) as f32;
    let variance = ((size_sum_sq / n) - (size_sum / n).powi(2)).max(0.0);
    let size_stddev = (variance.sqrt() as f32).max(0.01);

    let body_centroid = bucket_hist
        .iter()
        .max_by_key(|(_, &c)| c)
        .map(|(&key, _)| key as f32 / 10.0)
        .unwrap_or(size_mean);

    // --- Y-gap statistics ---
    let mut page_ys: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
    for pt in &sample {
        page_ys.entry(pt.page_index).or_default().push(pt.y);
    }

    let mut gaps: Vec<f32> = Vec::new();
    for ys in page_ys.values_mut() {
        ys.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        for window in ys.windows(2) {
            let gap = window[0] - window[1]; // both positive; descending sort
            if gap > 0.1 && gap < 200.0 {
                gaps.push(gap);
            }
        }
    }

    let gap_p90 = if gaps.is_empty() {
        28.0_f32
    } else {
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p90_idx = ((gaps.len() as f32 * 0.90) as usize).min(gaps.len().saturating_sub(1));
        gaps[p90_idx]
    };

    TypographicProfile {
        size_mean,
        size_stddev,
        body_centroid,
        gap_p90,
    }
}

// ---------------------------------------------------------------------------
// Phase 3 — Feature enrichment
// ---------------------------------------------------------------------------

/// Cached font-descriptor data for one font resource.
struct FontDescInfo {
    weight: u16,
    italic_angle: f32,
}

/// Look up `/FontDescriptor` data for every font declared on a page.
///
/// Clones dictionaries eagerly to avoid borrow-checker conflicts with `doc`.
fn get_page_font_descriptors(doc: &Document, page_oid: ObjectId) -> HashMap<String, FontDescInfo> {
    let mut map = HashMap::new();

    let page_dict = match doc.get_object(page_oid) {
        Ok(Object::Dictionary(d)) => d.clone(),
        _ => return map,
    };

    let resources = match page_dict.get(b"Resources") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Dictionary(d)) => d.clone(),
            _ => return map,
        },
        _ => return map,
    };

    let fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Dictionary(d)) => d.clone(),
            _ => return map,
        },
        _ => return map,
    };

    for (raw_key, val) in fonts.iter() {
        let key = String::from_utf8_lossy(raw_key).to_string();

        let font_dict = match val {
            Object::Dictionary(d) => d.clone(),
            Object::Reference(r) => match doc.get_object(*r) {
                Ok(Object::Dictionary(d)) => d.clone(),
                _ => continue,
            },
            _ => continue,
        };

        let mut weight: u16 = 0;
        let mut italic_angle: f32 = 0.0;

        // Bold indicator from BaseFont name.
        if let Ok(Object::Name(name)) = font_dict.get(b"BaseFont") {
            let lower = String::from_utf8_lossy(name).to_lowercase();
            if lower.contains("bold")
                || lower.contains("heavy")
                || lower.contains("black")
                || lower.ends_with("bd")
                || lower.ends_with("-bd")
            {
                weight = weight.max(700);
            }
        }

        // Precise weight and italic angle from /FontDescriptor.
        let fd_obj = match font_dict.get(b"FontDescriptor") {
            Ok(Object::Reference(r)) => doc.get_object(*r).ok().cloned(),
            Ok(Object::Dictionary(d)) => Some(Object::Dictionary(d.clone())),
            _ => None,
        };
        if let Some(Object::Dictionary(fd)) = fd_obj {
            match fd.get(b"FontWeight") {
                Ok(Object::Integer(w)) => weight = weight.max(*w as u16),
                Ok(Object::Real(w)) => weight = weight.max(*w as u16),
                _ => {}
            }
            match fd.get(b"ItalicAngle") {
                Ok(Object::Real(a)) => italic_angle = *a,
                Ok(Object::Integer(a)) => italic_angle = *a as f32,
                _ => {}
            }
        }

        map.insert(
            key,
            FontDescInfo {
                weight,
                italic_angle,
            },
        );
    }

    map
}

/// Detect the case pattern of a text string.
fn detect_case_pattern(text: &str) -> CasePattern {
    let alpha_words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .collect();

    if alpha_words.is_empty() {
        return CasePattern::Numeric;
    }

    // All-caps: every alpha char in every word is uppercase.
    if alpha_words.iter().all(|w| {
        w.chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_uppercase())
    }) {
        return CasePattern::AllCaps;
    }

    // Title case: first char of each word is uppercase.
    if alpha_words
        .iter()
        .all(|w| w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
    {
        return CasePattern::TitleCase;
    }

    // Sentence case: first word starts uppercase, rest start lowercase.
    let first_upper = alpha_words
        .first()
        .and_then(|w| w.chars().next())
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    let rest_lower = alpha_words[1..]
        .iter()
        .all(|w| w.chars().next().map(|c| c.is_lowercase()).unwrap_or(true));
    if first_upper && rest_lower {
        return CasePattern::SentenceCase;
    }

    CasePattern::Mixed
}

/// Build enriched feature vectors from raw `PositionedText` runs.
///
/// Performs per-page font-descriptor lookups (cached), computes Z-scores from
/// `profile`, determines case patterns, and computes the vertical gap above
/// each text run within its page.
pub fn build_text_features(
    doc: &Document,
    positioned: &[PositionedText],
    profile: &TypographicProfile,
) -> Vec<TextFeature> {
    if positioned.is_empty() {
        return vec![];
    }

    // Build page → ObjectId map.
    let page_oid_map: HashMap<u32, ObjectId> = doc
        .get_pages()
        .into_iter()
        .map(|(num, oid)| (num.saturating_sub(1), oid))
        .collect();

    // Cache font descriptors per page.
    let mut desc_cache: HashMap<u32, HashMap<String, FontDescInfo>> = HashMap::new();

    // Compute y_gap_above for each run (index in `positioned`).
    // Group runs by page, sort by Y descending, compute consecutive gaps.
    let mut page_ys: BTreeMap<u32, Vec<(f32, usize)>> = BTreeMap::new();
    for (i, pt) in positioned.iter().enumerate() {
        page_ys.entry(pt.page_index).or_default().push((pt.y, i));
    }
    for ys in page_ys.values_mut() {
        ys.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    }

    // gap_map[run_index] = vertical gap to the nearest text above on the same page.
    let mut gap_map: HashMap<usize, f32> = HashMap::with_capacity(positioned.len());
    for ys in page_ys.values() {
        // Top-most run gets a large sentinel (no text above it on the page).
        if let Some(&(_, top_idx)) = ys.first() {
            gap_map.insert(top_idx, 999.0);
        }
        for window in ys.windows(2) {
            // window[0] is higher on page (larger Y); window[1] is lower.
            let gap = (window[0].0 - window[1].0).max(0.0);
            gap_map.insert(window[1].1, gap);
        }
    }

    // Build features.
    let mut features = Vec::with_capacity(positioned.len());

    for (i, pt) in positioned.iter().enumerate() {
        // Fetch (and cache) font descriptors for this page.
        let page_descs = desc_cache.entry(pt.page_index).or_insert_with(|| {
            page_oid_map
                .get(&pt.page_index)
                .map(|&oid| get_page_font_descriptors(doc, oid))
                .unwrap_or_default()
        });

        let (font_weight, italic_angle) = page_descs
            .get(&pt.font_key)
            .map(|d| (d.weight, d.italic_angle))
            .unwrap_or((0, 0.0));

        // Z-score for font size.
        let size_z = if profile.size_stddev > 0.01 {
            (pt.font_size - profile.size_mean) / profile.size_stddev
        } else {
            0.0
        };

        // Bit-flag assembly.
        let mut flags: TextFlags = 0;

        if font_weight > 600 {
            flags |= FLAG_BOLD;
        }
        if italic_angle < -5.0 {
            flags |= FLAG_ITALIC;
        }
        if size_z > 3.0 {
            flags |= FLAG_LARGE_FONT;
        } else if size_z >= 1.5 {
            flags |= FLAG_MED_FONT;
        } else if size_z < -1.5 {
            flags |= FLAG_SMALL_FONT;
        }

        let case_pattern = detect_case_pattern(&pt.text);
        match case_pattern {
            CasePattern::AllCaps => flags |= FLAG_ALL_CAPS,
            CasePattern::TitleCase => flags |= FLAG_TITLE_CASE,
            _ => {}
        }

        let y_gap_above = gap_map.get(&i).copied().unwrap_or(0.0);
        if y_gap_above >= profile.gap_p90 {
            flags |= FLAG_ISOLATED;
        }

        features.push(TextFeature {
            page_index: pt.page_index,
            x: pt.x,
            y: pt.y,
            effective_font_size: pt.font_size,
            size_z,
            font_key: pt.font_key.clone(),
            text: pt.text.clone(),
            flags,
            font_weight,
            italic_angle,
            y_gap_above,
            case_pattern,
        });
    }

    features
}

// ---------------------------------------------------------------------------
// Phase 4 — Multi-heuristic classification with Bayesian boosts
// ---------------------------------------------------------------------------

/// Classify enriched text features into structural anchors.
///
/// Optionally accepts `toc_entries` (`(title, printed_page)`) and a
/// `predicted_offset` to apply Bayesian confidence boosts when an anchor's
/// text fuzzy-matches a TOC entry and/or falls at the predicted physical page.
pub fn classify_features(
    features: &[TextFeature],
    toc_entries: &[(String, u32)],
    predicted_offset: Option<i32>,
) -> Vec<LayoutAnchor> {
    let mut anchors = Vec::new();

    for ft in features {
        let trimmed = ft.text.trim();
        if trimmed.len() < 2 {
            continue;
        }

        let bold = ft.is(FLAG_BOLD);
        let isolated = ft.is(FLAG_ISOLATED);
        let large = ft.is(FLAG_LARGE_FONT);
        let medium = ft.is(FLAG_MED_FONT);
        let small = ft.is(FLAG_SMALL_FONT);
        let all_caps = ft.is(FLAG_ALL_CAPS);

        // --- Classification rules (first match wins) ---
        let classification: Option<(AnchorKind, f32)> = if large && bold && isolated {
            // Largest confidence: distinctly large + bold + isolated.
            let base = (0.50 + ft.size_z * 0.05).clamp(0.70, 0.95);
            Some((AnchorKind::ChapterHeading, base))
        } else if large && isolated {
            // Large + isolated, no bold evidence.
            let base = (0.40 + ft.size_z * 0.04).clamp(0.60, 0.90);
            Some((AnchorKind::ChapterHeading, base))
        } else if (bold || all_caps) && isolated && !large && !small {
            // Bold/all-caps + isolated at body or medium size → section or numbered.
            if is_numbered_heading(trimmed) {
                Some((AnchorKind::NumberedHeading, 0.85))
            } else {
                Some((AnchorKind::SectionHeading, 0.75))
            }
        } else if bold && !isolated {
            // Bold but not isolated — inline emphasis; skip.
            None
        } else if is_toc_entry(trimmed) {
            Some((AnchorKind::TocEntry, 0.70))
        } else if medium && is_numbered_heading(trimmed) {
            Some((AnchorKind::NumberedHeading, 0.80))
        } else {
            None
        };

        let Some((kind, mut confidence)) = classification else {
            continue;
        };

        // --- Bayesian confidence boosts ---
        'toc: for (toc_title, printed_page) in toc_entries {
            let sim = strsim::normalized_levenshtein(trimmed, toc_title.trim());
            if sim >= 0.80 {
                confidence += 0.20;
                if let Some(offset) = predicted_offset {
                    let expected = (*printed_page as i32).saturating_add(offset) as u32;
                    if ft.page_index == expected {
                        confidence += 0.10;
                    }
                }
                break 'toc;
            }
        }

        confidence = confidence.clamp(0.0, 1.0);
        if confidence < 0.40 {
            continue;
        }

        anchors.push(LayoutAnchor {
            page_index: ft.page_index,
            kind,
            text: trimmed.to_string(),
            font_size: ft.effective_font_size,
            y: ft.y,
            confidence,
        });
    }

    anchors
}

// ---------------------------------------------------------------------------
// Legacy clustering-only anchor detection (backward compat)
// ---------------------------------------------------------------------------

/// Detect structural anchors from positioned text using font-size clustering.
///
/// This is the legacy path (no font descriptor or spatial data).  The full
/// feature-vector pipeline is used by [`analyze_layout`].
// ---------------------------------------------------------------------------
// TOC / page-number helpers
// ---------------------------------------------------------------------------

/// Detect pages that are likely part of a Table of Contents.
///
/// A page is a TOC page if ≥ 3 of its text runs match the TOC-entry pattern
/// AND at least 30 % of all runs on that page are TOC entries.
pub fn detect_toc_pages(positioned: &[PositionedText]) -> Vec<u32> {
    let mut toc_counts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut total_counts: BTreeMap<u32, u32> = BTreeMap::new();

    for pt in positioned {
        *total_counts.entry(pt.page_index).or_insert(0) += 1;
        if is_toc_entry(pt.text.trim()) {
            *toc_counts.entry(pt.page_index).or_insert(0) += 1;
        }
    }

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
/// Returns `(page_index, detected_number)` for isolated 1–4-digit numbers
/// found in the top or bottom 10 % of the page height.
pub fn detect_page_numbers(positioned: &[PositionedText], page_height: f32) -> Vec<(u32, u32)> {
    let margin = page_height * 0.10;
    let mut results = Vec::new();

    for pt in positioned {
        let in_footer = pt.y < margin;
        let in_header = pt.y > (page_height - margin);
        if !in_footer && !in_header {
            continue;
        }
        let trimmed = pt.text.trim();
        if !trimmed.is_empty() && trimmed.len() <= 4 && trimmed.chars().all(|c| c.is_ascii_digit())
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

/// Full layout analysis pipeline.
///
/// Runs extraction → profiling → feature enrichment → classification.
/// Returns a [`LayoutResult`] with anchors, clusters, and feature vectors.
pub fn analyze_layout(doc: &Document, path: &str) -> LayoutResult {
    let pages = doc.get_pages();
    let total_pages = pages.len() as u32;

    // Extract positioned text with Tm-scale-corrected font sizes.
    let positioned = extract_all_positioned(doc);

    // Histogram for cluster-based role assignment (pipeline.rs compatibility).
    let histogram = build_font_histogram(doc).unwrap_or_default();
    let raw_clusters = cluster_font_sizes(&histogram, 6);
    let font_clusters = assign_roles(&raw_clusters);

    // Typographic profile from first 50 pages.
    let profile = build_typographic_profile(&positioned, 50);

    // Feature enrichment.
    let features = build_text_features(doc, &positioned, &profile);

    // Classification (no TOC data / predicted offset at this stage).
    let anchors = classify_features(&features, &[], None);

    LayoutResult {
        path: path.to_string(),
        total_pages,
        body_font_size: profile.body_centroid,
        anchors,
        font_clusters,
        features,
    }
}

// ---------------------------------------------------------------------------
// Pattern helpers
// ---------------------------------------------------------------------------

/// Check if text matches a numbered heading pattern.
fn is_numbered_heading(text: &str) -> bool {
    let lower = text.to_lowercase();

    if lower.starts_with("chapter ")
        || lower.starts_with("part ")
        || lower.starts_with("section ")
        || lower.starts_with("appendix ")
    {
        return true;
    }

    let first_word = text.split_whitespace().next().unwrap_or("");
    if !first_word.is_empty() && text.split_whitespace().count() > 1 {
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
///
/// False-positive guard: the title part must contain at least one word with
/// ≥ 2 alphabetic characters, filtering out math matrices ("a 3 x 3"),
/// pixel arrays, and transformation matrices ("1 0 0 1 0 …").
fn is_toc_entry(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 3 {
        return false;
    }

    if let Some(last_word) = trimmed.split_whitespace().last() {
        if last_word.len() <= 4
            && last_word.chars().all(|c| c.is_ascii_digit())
            && last_word.parse::<u32>().is_ok()
        {
            // Leader dots, tabs, or multiple spaces → clear TOC pattern.
            let prefix = trimmed.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
            if prefix.contains("..") || prefix.contains("  ") || prefix.contains('\t') {
                return true;
            }
            // Text part long enough to be a title — but requires a real word.
            let text_part = trimmed[..trimmed.len() - last_word.len()].trim();
            let has_alpha_word = text_part
                .split_whitespace()
                .any(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2);
            if text_part.len() >= 5 && has_alpha_word {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Pixel-density stubs (Scanned Tier — deferred to OCR milestone)
// ---------------------------------------------------------------------------

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
        assert!(!is_numbered_heading("42"));
    }

    #[test]
    fn toc_entry_patterns() {
        // Positive cases.
        assert!(is_toc_entry("Chapter 1 .... 15"));
        assert!(is_toc_entry("Introduction ............... 1"));
        assert!(is_toc_entry("Methods   42"));
        assert!(is_toc_entry("Related Work\t10"));
        assert!(is_toc_entry("Background and Motivation 100"));
        // Too short.
        assert!(!is_toc_entry("42"));
        assert!(!is_toc_entry("ab"));
        assert!(!is_toc_entry(""));
        // False-positive guards: math expressions / pixel arrays.
        assert!(!is_toc_entry("a 3 x 3"));
        assert!(!is_toc_entry("1 0 0 1 0"));
        assert!(!is_toc_entry("186 168 130 3"));
        assert!(!is_toc_entry("a b c 10"));
    }

    #[test]
    fn detect_page_numbers_footer() {
        let positioned = vec![
            PositionedText {
                page_index: 0,
                x: 300.0,
                y: 30.0,
                font_size: 10.0,
                font_key: "F1".into(),
                text: "42".into(),
            },
            PositionedText {
                page_index: 0,
                x: 100.0,
                y: 400.0,
                font_size: 12.0,
                font_key: "F1".into(),
                text: "Some body text".into(),
            },
            PositionedText {
                page_index: 1,
                x: 300.0,
                y: 760.0,
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

    fn make_test_doc_with_content(content: &[u8]) -> (lopdf::Document, ObjectId) {
        use lopdf::{Dictionary, Stream};
        let mut doc = lopdf::Document::with_version("1.7");
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
        (doc, page_id)
    }

    #[test]
    fn text_matrix_tracking() {
        let (doc, page_id) =
            make_test_doc_with_content(b"BT /F1 14 Tf 72 700 Td (Chapter 1) Tj ET");
        let positioned = extract_positioned_text(&doc, page_id, 0);
        assert_eq!(positioned.len(), 1);
        assert_eq!(positioned[0].text, "Chapter 1");
        assert!((positioned[0].x - 72.0).abs() < 0.1);
        assert!((positioned[0].y - 700.0).abs() < 0.1);
        assert!((positioned[0].font_size - 14.0).abs() < 0.1);
    }

    #[test]
    fn text_matrix_scale_tracking() {
        // Nominal Tf=1pt, Tm matrix scales by 14 → effective size = 14pt.
        let (doc, page_id) =
            make_test_doc_with_content(b"BT /F1 1 Tf 14 0 0 14 72 700 Tm (Heading) Tj ET");
        let positioned = extract_positioned_text(&doc, page_id, 0);
        assert_eq!(positioned.len(), 1, "expected one text run");
        assert!(
            (positioned[0].font_size - 14.0).abs() < 0.5,
            "expected effective size ~14pt, got {}",
            positioned[0].font_size
        );
    }

    #[test]
    fn typographic_profile_body_centroid() {
        let mut positioned = Vec::new();
        // 1000 body-text runs at 12pt across 50 pages.
        for i in 0..1000_u32 {
            positioned.push(PositionedText {
                page_index: i / 20,
                x: 72.0,
                y: 700.0 - (i % 20) as f32 * 30.0,
                font_size: 12.0,
                font_key: "F1".into(),
                text: "body text word".into(),
            });
        }
        // 10 heading runs at 18pt.
        for i in 0..10_u32 {
            positioned.push(PositionedText {
                page_index: i,
                x: 72.0,
                y: 750.0,
                font_size: 18.0,
                font_key: "F2".into(),
                text: "Chapter Heading".into(),
            });
        }
        let profile = build_typographic_profile(&positioned, 50);
        assert!(
            (profile.body_centroid - 12.0).abs() < 0.5,
            "expected body centroid ~12pt, got {}",
            profile.body_centroid
        );
    }

    #[test]
    fn feature_z_score_flags() {
        let profile = TypographicProfile {
            size_mean: 10.0,
            size_stddev: 1.0,
            body_centroid: 10.0,
            gap_p90: 28.0,
        };
        // z = 1.5 → FLAG_MED_FONT
        let z_med = (11.5 - profile.size_mean) / profile.size_stddev;
        assert!((z_med - 1.5).abs() < 0.01);
        assert!(
            (1.5..3.0).contains(&z_med),
            "z={} should be in [1.5, 3.0)",
            z_med
        );
        // z = 4.0 → FLAG_LARGE_FONT
        let z_large = (14.0 - profile.size_mean) / profile.size_stddev;
        assert!(z_large > 3.0, "z={} should be > 3.0", z_large);
    }

    #[test]
    fn detect_case_pattern_variants() {
        assert_eq!(detect_case_pattern("INTRODUCTION"), CasePattern::AllCaps);
        assert_eq!(detect_case_pattern("Chapter One"), CasePattern::TitleCase);
        assert_eq!(
            detect_case_pattern("This is body text"),
            CasePattern::SentenceCase
        );
        assert_eq!(detect_case_pattern("123 456"), CasePattern::Numeric);
    }

    #[test]
    fn detect_toc_pages_finds_dense_pages() {
        let mut positioned = Vec::new();
        for i in 0..8_u32 {
            positioned.push(PositionedText {
                page_index: 2,
                x: 72.0,
                y: 700.0 - i as f32 * 20.0,
                font_size: 12.0,
                font_key: "F1".into(),
                text: format!("Chapter {} .... {}", i + 1, (i + 1) * 10),
            });
        }
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
