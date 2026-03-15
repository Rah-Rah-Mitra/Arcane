//! Outline recovery pipeline orchestrator.
//!
//! Composes the probe, layout, clustering, offset, and injection modules
//! into a single end-to-end recovery flow for the enhanced `recover-outline`
//! command.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use lopdf::Document;
use serde::{Deserialize, Serialize};

use super::clustering::{assign_roles, cluster_font_sizes, FontCluster};
use super::heuristics::{self, HeadingCandidate};
use super::layout::{self, LayoutResult};
use super::offset::{self, OffsetResult};
use super::probe::{self, DocumentKind, ProbeResult};

// ---------------------------------------------------------------------------
// Configuration & result types
// ---------------------------------------------------------------------------

/// Configuration for the recovery pipeline.
pub struct RecoveryConfig {
    /// Font-size ratio above body text to classify as heading.
    pub min_font_ratio: f32,
    /// Maximum heading depth (1 = chapters only, 2 = chapters + sections).
    pub max_depth: u32,
    /// User-specified TOC page range (0-based, inclusive).
    pub toc_pages: Option<(u32, u32)>,
    /// Preview only — do not inject.
    pub dry_run: bool,
    /// Whether to inject outlines into the document.
    pub inject: bool,
    /// Minimum fuzzy-match similarity for verification.
    pub fuzzy_threshold: f64,
}

/// Full pipeline result.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecoveryResult {
    /// PDF classification result.
    pub probe: ProbeResult,
    /// Layout analysis result (if text-based).
    pub layout: Option<LayoutResult>,
    /// Page offset result (if determinable).
    pub offset: Option<OffsetResult>,
    /// Detected heading candidates.
    pub headings: Vec<HeadingInfo>,
    /// Final chapter map (page_index → title).
    pub chapter_map: BTreeMap<u32, String>,
    /// Verification results for each heading.
    pub verification: Vec<VerificationEntry>,
    /// Number of outline entries injected (None if dry-run or skipped).
    pub injected_count: Option<usize>,
}

/// Serialisable heading info (subset of HeadingCandidate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadingInfo {
    pub page_index: u32,
    pub font_size: f32,
    pub text: String,
    pub y_position: Option<f32>,
    pub depth_level: u32,
}

impl From<&HeadingCandidate> for HeadingInfo {
    fn from(h: &HeadingCandidate) -> Self {
        HeadingInfo {
            page_index: h.page_index,
            font_size: h.font_size,
            text: h.text.clone(),
            y_position: h.y_position,
            depth_level: h.depth_level,
        }
    }
}

/// Verification of a single heading against actual page text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEntry {
    /// The heading text being verified.
    pub heading_text: String,
    /// Physical page index where the heading should appear.
    pub physical_page: u32,
    /// Snippet of text found on that page.
    pub page_text_snippet: String,
    /// Fuzzy-match similarity score (0.0 to 1.0).
    pub similarity: f64,
    /// Whether the heading passed verification.
    pub verified: bool,
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Run the full tiered outline recovery pipeline.
///
/// 1. **Probe** — classify document (text vs scanned)
/// 2. **Route** — Tier 1 (heuristic) for text, report "OCR needed" for scanned
/// 3. **Extract** — headings via enhanced clustering + positions
/// 4. **Offset** — calculate logical-to-physical page delta
/// 5. **Verify** — fuzzy-match headings against page text
/// 6. **Inject** — write hierarchical `/Outlines` tree (unless dry-run)
pub fn recover_outline(
    doc: &mut Document,
    path: &str,
    config: &RecoveryConfig,
) -> Result<RecoveryResult> {
    // Phase 1: Probe.
    let probe_result = probe::probe(doc, path);

    // Phase 2: Route.
    match probe_result.document_kind {
        DocumentKind::Scanned => {
            #[cfg(feature = "ocr")]
            {
                tracing::info!("Scanned PDF — running Tier 2 OCR on all pages");
                let all_pages: Vec<u32> = (0..probe_result.total_pages).collect();
                match tier2_ocr(path, &all_pages, config, None) {
                    Ok(h) if !h.is_empty() => {
                        // Continue the pipeline with OCR-extracted headings below.
                        // We break out of the match by falling through to Phase 3
                        // via a separate code path.
                        return finish_pipeline(doc, path, config, probe_result, None, h);
                    }
                    Ok(_) => {
                        tracing::warn!("Tier 2 OCR produced no headings for scanned PDF");
                    }
                    Err(e) => {
                        tracing::warn!("Tier 2 OCR failed: {e:#}");
                    }
                }
                return Ok(RecoveryResult {
                    probe: probe_result,
                    layout: None,
                    offset: None,
                    headings: vec![],
                    chapter_map: BTreeMap::new(),
                    verification: vec![],
                    injected_count: None,
                });
            }
            #[cfg(not(feature = "ocr"))]
            {
                tracing::warn!(
                    "Scanned PDF detected — OCR not compiled in. \
                     Rebuild with `cargo build --features ocr` to process this file."
                );
                return Ok(RecoveryResult {
                    probe: probe_result,
                    layout: None,
                    offset: None,
                    headings: vec![],
                    chapter_map: BTreeMap::new(),
                    verification: vec![],
                    injected_count: None,
                });
            }
        }
        DocumentKind::Empty => {
            anyhow::bail!("PDF has no usable content");
        }
        DocumentKind::TextBased | DocumentKind::Mixed => {
            // Proceed with Tier 1 heuristic extraction.
        }
    }

    // Phase 3: Tier 1 — heuristic extraction with enhanced clustering.
    let (headings, layout_result) = tier1_heuristic(doc, config)?;

    if headings.is_empty() {
        return Ok(RecoveryResult {
            probe: probe_result,
            layout: Some(layout_result),
            offset: None,
            headings: vec![],
            chapter_map: BTreeMap::new(),
            verification: vec![],
            injected_count: None,
        });
    }

    // Tier 2 quality fallback: if Tier 1 text is mostly garbled (< 50% alpha),
    // the PDF has a broken font encoding — re-run on the same pages via OCR.
    // Uses `let headings = ...` shadowing so no `mut` is needed on the binding.
    #[cfg(feature = "ocr")]
    let headings = {
        let quality = heading_text_quality(&headings);
        if quality < 0.5 {
            tracing::info!(
                "Tier 1 text quality {:.0}% — falling back to OCR on {} candidate pages",
                quality * 100.0,
                headings.len()
            );
            let pages: Vec<u32> = headings.iter().map(|h| h.page_index).collect();
            match tier2_ocr(path, &pages, config, Some(&headings)) {
                Ok(ocr_headings) if !ocr_headings.is_empty() => ocr_headings,
                Ok(_) => {
                    tracing::warn!("Tier 2 OCR produced no headings — keeping Tier 1 results");
                    headings
                }
                Err(e) => {
                    tracing::warn!("Tier 2 OCR failed: {e:#} — keeping Tier 1 results");
                    headings
                }
            }
        } else {
            headings
        }
    };

    finish_pipeline(
        doc,
        path,
        config,
        probe_result,
        Some(layout_result),
        headings,
    )
}

// ---------------------------------------------------------------------------
// Shared pipeline tail (phases 4–6)
// ---------------------------------------------------------------------------

/// Complete phases 4–6 of the pipeline (offset, verify, inject) and return
/// the final [`RecoveryResult`].
///
/// Extracted so that both the TextBased/Mixed path and the Scanned/OCR path
/// can share the same logic without duplication.
fn finish_pipeline(
    doc: &mut Document,
    _path: &str,
    config: &RecoveryConfig,
    probe_result: ProbeResult,
    layout_result: Option<LayoutResult>,
    headings: Vec<HeadingCandidate>,
) -> Result<RecoveryResult> {
    if headings.is_empty() {
        return Ok(RecoveryResult {
            probe: probe_result,
            layout: layout_result,
            offset: None,
            headings: vec![],
            chapter_map: BTreeMap::new(),
            verification: vec![],
            injected_count: None,
        });
    }

    // Phase 4: Offset calculation.
    let offset_result = offset::calculate_offset(doc, layout_result.as_ref(), config.toc_pages);

    // Phase 5: Verify headings against page text.
    let verification = verify_headings(doc, &headings, config.fuzzy_threshold);

    // Build the chapter map from verified headings.
    let chapter_map = build_chapter_map(&headings);

    // Build hierarchical entries for injection.
    let hierarchical_entries: Vec<(u32, String, u32)> = headings
        .iter()
        .map(|h| (h.page_index, h.text.clone(), h.depth_level))
        .collect();

    let heading_infos: Vec<HeadingInfo> = headings.iter().map(HeadingInfo::from).collect();

    // Phase 6: Inject (unless dry-run).
    let injected_count = if config.inject && !config.dry_run && !chapter_map.is_empty() {
        let max_depth = headings.iter().map(|h| h.depth_level).max().unwrap_or(1);
        let count = if max_depth > 1 {
            heuristics::inject_hierarchical_outlines(doc, &hierarchical_entries)
                .context("failed to inject hierarchical outlines")?
        } else {
            let flat_map: BTreeMap<u32, String> = chapter_map.clone();
            heuristics::inject_outlines(doc, &flat_map).context("failed to inject outlines")?
        };
        Some(count)
    } else {
        None
    };

    Ok(RecoveryResult {
        probe: probe_result,
        layout: layout_result,
        offset: offset_result,
        headings: heading_infos,
        chapter_map,
        verification,
        injected_count,
    })
}

// ---------------------------------------------------------------------------
// Heading text quality helper (always compiled)
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
/// Returns the ratio of alphabetic characters across all heading texts (0.0–1.0).
///
/// Values below ~0.5 indicate garbled font encoding (e.g. non-standard
/// `ToUnicode` mappings that map bytes to `~` or null bytes).  The OCR tier
/// uses this as its trigger threshold.
fn heading_text_quality(headings: &[HeadingCandidate]) -> f32 {
    let total: usize = headings.iter().map(|h| h.text.len()).sum();
    if total == 0 {
        return 0.0;
    }
    let alpha: usize = headings
        .iter()
        .flat_map(|h| h.text.chars())
        .filter(|c| c.is_alphabetic())
        .count();
    alpha as f32 / total as f32
}

// ---------------------------------------------------------------------------
// Tier 2: OCR-based extraction (feature-gated)
// ---------------------------------------------------------------------------

/// Tier 2 extraction — render pages via pdfium and recognise text with oar-ocr.
///
/// Only compiled when the `ocr` Cargo feature is enabled.
///
/// When `tier1_headings` is provided, OCR results are filtered to only keep
/// regions whose font size is within 50% of a Tier 1 heading on the same page.
/// This uses Tier 1's correct font-size discrimination while replacing the
/// garbled text with OCR-recognised text.
#[cfg(feature = "ocr")]
fn tier2_ocr(
    path: &str,
    candidate_pages: &[u32],
    _config: &RecoveryConfig,
    tier1_headings: Option<&[HeadingCandidate]>,
) -> Result<Vec<HeadingCandidate>> {
    use super::ocr;
    let positioned = ocr::extract_headings_ocr(
        std::path::Path::new(path),
        candidate_pages,
        150, // 150 DPI: ~0.3 s/page, good enough for headings
    )?;

    // Build a per-page set of Tier 1 heading font sizes for filtering.
    let tier1_sizes: std::collections::HashMap<u32, Vec<f32>> = tier1_headings
        .map(|hs| {
            let mut map: std::collections::HashMap<u32, Vec<f32>> =
                std::collections::HashMap::new();
            for h in hs {
                map.entry(h.page_index).or_default().push(h.font_size);
            }
            map
        })
        .unwrap_or_default();

    let headings = positioned
        .into_iter()
        .filter(|pt| !pt.text.is_empty())
        .filter(|pt| {
            // If we have Tier 1 reference sizes, only keep OCR regions whose
            // font size is within 50% of some Tier 1 heading on the same page.
            if let Some(sizes) = tier1_sizes.get(&pt.page_index) {
                sizes.iter().any(|&s| {
                    let ratio = pt.font_size / s;
                    (0.5..=1.5).contains(&ratio)
                })
            } else {
                // No Tier 1 reference for this page (e.g. scanned PDF) — keep all.
                true
            }
        })
        .map(|pt| HeadingCandidate {
            page_index: pt.page_index,
            font_size: pt.font_size,
            text: pt.text,
            y_position: Some(pt.y),
            // Coarse depth assignment: larger boxes → top-level headings.
            depth_level: if pt.font_size > 20.0 { 1 } else { 2 },
        })
        .collect();
    Ok(headings)
}

// ---------------------------------------------------------------------------
// Tier 1: heuristic extraction
// ---------------------------------------------------------------------------

/// Tier 1 extraction for text-based PDFs.
///
/// Uses clustering + position-aware extraction to produce heading candidates
/// with depth levels and Y-coordinates.
fn tier1_heuristic(
    doc: &Document,
    config: &RecoveryConfig,
) -> Result<(Vec<HeadingCandidate>, LayoutResult)> {
    // Run layout analysis.
    let layout_result = layout::analyze_layout(doc, "");

    // Get font clusters.
    let histogram = heuristics::build_font_histogram(doc).unwrap_or_default();
    let raw_clusters = cluster_font_sizes(&histogram, 6);
    let clusters = assign_roles(&raw_clusters);

    // Extract positioned text for heading detection.
    let positioned = layout::extract_all_positioned(doc);

    // Build headings from positioned text using cluster-based classification.
    let mut headings: Vec<HeadingCandidate> = Vec::new();

    for pt in &positioned {
        let role = find_role(pt.font_size, &clusters);

        let depth_level = match role {
            Some(super::clustering::FontRole::Heading1) => 1,
            Some(super::clustering::FontRole::Heading2) => 2,
            Some(super::clustering::FontRole::Heading3) => 3,
            _ => continue,
        };

        // Only include up to max_depth.
        if depth_level > config.max_depth {
            continue;
        }

        let trimmed = pt.text.trim();
        // Filter out trivial text.
        if trimmed.len() <= 1
            || trimmed
                .chars()
                .all(|c| c.is_numeric() || c == '.' || c == ' ')
        {
            continue;
        }

        headings.push(HeadingCandidate {
            page_index: pt.page_index,
            font_size: pt.font_size,
            text: trimmed.to_string(),
            y_position: Some(pt.y),
            depth_level,
        });
    }

    // If cluster-based extraction found nothing, fall back to the original
    // threshold-based approach (backward compatibility).
    if headings.is_empty() {
        let fallback = heuristics::extract_headings(doc, config.min_font_ratio, config.max_depth);
        headings = fallback;
    }

    // Merge adjacent heading runs on the same page with same depth.
    headings = merge_adjacent_headings(headings);

    Ok((headings, layout_result))
}

/// Find the font role for a given size by nearest cluster centroid.
fn find_role(size: f32, clusters: &[FontCluster]) -> Option<super::clustering::FontRole> {
    clusters
        .iter()
        .min_by_key(|c| ((c.centroid - size).abs() * 100.0) as u32)
        .map(|c| c.role)
}

/// Merge adjacent heading candidates on the same page with the same depth.
fn merge_adjacent_headings(headings: Vec<HeadingCandidate>) -> Vec<HeadingCandidate> {
    if headings.len() <= 1 {
        return headings;
    }

    let mut merged: Vec<HeadingCandidate> = Vec::new();

    let mut i = 0;
    while i < headings.len() {
        let mut current = headings[i].clone();
        let mut j = i + 1;

        // Merge following entries on the same page with same depth and similar font size.
        while j < headings.len()
            && headings[j].page_index == current.page_index
            && headings[j].depth_level == current.depth_level
            && (headings[j].font_size - current.font_size).abs() < 0.5
        {
            current.text = format!("{} {}", current.text, headings[j].text);
            j += 1;
        }

        // Clean up whitespace.
        current.text = current
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !current.text.is_empty() {
            merged.push(current);
        }

        i = j;
    }

    merged
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Verify heading candidates against actual page text.
fn verify_headings(
    doc: &Document,
    headings: &[HeadingCandidate],
    threshold: f64,
) -> Vec<VerificationEntry> {
    headings
        .iter()
        .map(|h| {
            let page_text = doc
                .extract_text(&[h.page_index + 1]) // 1-based
                .unwrap_or_default();

            let snippet = page_text
                .lines()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect::<String>();

            let similarity = if page_text.is_empty() {
                0.0
            } else {
                best_line_similarity(&h.text, &page_text)
            };

            VerificationEntry {
                heading_text: h.text.clone(),
                physical_page: h.page_index,
                page_text_snippet: snippet,
                similarity,
                verified: similarity >= threshold,
            }
        })
        .collect()
}

/// Find best line-level similarity between needle and haystack.
fn best_line_similarity(needle: &str, haystack: &str) -> f64 {
    let needle_lower = needle.to_lowercase();

    if haystack.to_lowercase().contains(&needle_lower) {
        return 1.0;
    }

    haystack
        .lines()
        .map(|line| strsim::normalized_levenshtein(&needle_lower, &line.trim().to_lowercase()))
        .fold(0.0_f64, f64::max)
}

// ---------------------------------------------------------------------------
// Chapter map building
// ---------------------------------------------------------------------------

/// Build a chapter map from headings, keeping only the largest heading per page.
fn build_chapter_map(headings: &[HeadingCandidate]) -> BTreeMap<u32, String> {
    let mut map: BTreeMap<u32, &HeadingCandidate> = BTreeMap::new();
    for h in headings {
        map.entry(h.page_index)
            .and_modify(|existing| {
                // Prefer lower depth_level (chapter > section) or larger font.
                if h.depth_level < existing.depth_level
                    || (h.depth_level == existing.depth_level && h.font_size > existing.font_size)
                {
                    *existing = h;
                }
            })
            .or_insert(h);
    }
    map.into_iter()
        .map(|(page, h)| (page, h.text.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_adjacent_same_page() {
        let headings = vec![
            HeadingCandidate {
                page_index: 0,
                font_size: 18.0,
                text: "Chapter".into(),
                y_position: Some(700.0),
                depth_level: 1,
            },
            HeadingCandidate {
                page_index: 0,
                font_size: 18.0,
                text: "One".into(),
                y_position: Some(680.0),
                depth_level: 1,
            },
        ];

        let merged = merge_adjacent_headings(headings);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Chapter One");
    }

    #[test]
    fn merge_different_pages_kept_separate() {
        let headings = vec![
            HeadingCandidate {
                page_index: 0,
                font_size: 18.0,
                text: "Chapter 1".into(),
                y_position: None,
                depth_level: 1,
            },
            HeadingCandidate {
                page_index: 5,
                font_size: 18.0,
                text: "Chapter 2".into(),
                y_position: None,
                depth_level: 1,
            },
        ];

        let merged = merge_adjacent_headings(headings);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn build_chapter_map_prefers_lower_depth() {
        let headings = vec![
            HeadingCandidate {
                page_index: 0,
                font_size: 18.0,
                text: "Chapter 1".into(),
                y_position: None,
                depth_level: 1,
            },
            HeadingCandidate {
                page_index: 0,
                font_size: 14.0,
                text: "Section 1.1".into(),
                y_position: None,
                depth_level: 2,
            },
        ];

        let map = build_chapter_map(&headings);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&0], "Chapter 1"); // chapter wins over section
    }

    #[test]
    fn best_line_similarity_exact_match() {
        let sim = best_line_similarity("Introduction", "Introduction\nSome body text");
        assert!(sim >= 0.99);
    }

    #[test]
    fn best_line_similarity_no_match() {
        let sim = best_line_similarity("Appendix Z", "Completely unrelated text\nNothing here");
        assert!(sim < 0.6);
    }

    #[test]
    fn heading_info_conversion() {
        let h = HeadingCandidate {
            page_index: 5,
            font_size: 18.0,
            text: "Chapter 3".into(),
            y_position: Some(700.0),
            depth_level: 1,
        };
        let info = HeadingInfo::from(&h);
        assert_eq!(info.page_index, 5);
        assert_eq!(info.depth_level, 1);
        assert_eq!(info.text, "Chapter 3");
    }
}
