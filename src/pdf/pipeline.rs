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
use super::offset::{self, OffsetMethod, OffsetResult};
use super::probe::{self, DocumentKind, ProbeResult};
use super::seed::{self, ResolvedSeed, SeedEntry};
// SeedEntry is used in recover_outline_seeded's public parameter.

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
    /// ±N page tolerance window when locating seeds in the target PDF.
    pub page_shift_tolerance: u32,
    /// Search range for the seed-offset vote.  Independent of `page_shift_tolerance`
    /// so that large physical offsets (e.g. 20+ pages of front matter) are found
    /// automatically without widening the per-seed page-search window.
    pub offset_tolerance: u32,
    /// User-supplied base offset (physical_0based = logical_page - 1 + offset).
    /// When set, bypasses seed-offset voting entirely.
    pub user_offset: Option<i32>,
    /// Per-segment page pivot points (logical_1based, physical_1based).
    /// After seed resolution, Estimated seeds use the local offset of the
    /// nearest anchor at or before their logical position.
    pub user_anchors: Vec<(u32, u32)>,
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
    /// Seed verification results (present when --seed-pdf or --seed-file was used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_verification: Option<Vec<ResolvedSeed>>,
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
/// 2. **Route** — Tier 1 (heuristic) for text, report unsupported for scanned
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
            tracing::warn!("Scanned PDF detected — outline recovery not supported for scanned documents.");
            return Ok(RecoveryResult {
                probe: probe_result,
                layout: None,
                offset: None,
                headings: vec![],
                chapter_map: BTreeMap::new(),
                verification: vec![],
                injected_count: None,
                seed_verification: None,
            });
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
            seed_verification: None,
        });
    }

    finish_pipeline(
        doc,
        path,
        config,
        probe_result,
        Some(layout_result),
        headings,
        None,
    )
}

// ---------------------------------------------------------------------------
// Seeded pipeline entry point
// ---------------------------------------------------------------------------

/// Run the seeded outline recovery pipeline.
///
/// Uses `seeds` (from `--seed-pdf` or `--seed-file`) as ground-truth chapter
/// titles, bypassing heuristic heading *detection*.  Text search
/// is used to *verify* which physical page each seed title lands on.
///
/// Flow:
/// 1. **Probe** — classify document type.
/// 2. **Offset from seeds** — vote over candidate offsets; fall back to
///    standard detection if the vote is inconclusive.
/// 3. **Resolve** — locate each seed in the target PDF via text extraction.
/// 4. **Convert** — seeds → `HeadingCandidate` list.
/// 5. **Finish** — phases 5 & 6 (verify + inject) via `finish_pipeline`.
pub fn recover_outline_seeded(
    doc: &mut Document,
    path: &str,
    config: &RecoveryConfig,
    seeds: Vec<SeedEntry>,
) -> Result<RecoveryResult> {
    // Phase 1: Probe.
    let probe_result = probe::probe(doc, path);

    // Phase 2: Offset from seeds.
    // If the user supplied --page-one, use it directly (confidence 1.0) and skip
    // the voting step entirely.  Otherwise run the consensus vote.
    // Use a lower internal threshold (0.15) for the vote so that even partially-
    // readable text produces a signal.  The full fuzzy_threshold is reserved for
    // seed *verification* (resolve_seeds / verify_seeds_ocr).
    let (offset, seed_offset_result) = if let Some(user_off) = config.user_offset {
        tracing::info!("Using user-supplied offset: {user_off} (--page-one)");
        let result = OffsetResult {
            offset: user_off,
            confidence: 1.0,
            method: OffsetMethod::UserSupplied,
            evidence: vec![],
        };
        (user_off, Some(result))
    } else {
        let vote_threshold = config.fuzzy_threshold.min(0.15);
        match seed::calculate_offset_from_seeds(
            &seeds,
            doc,
            vote_threshold,
            config.offset_tolerance,
        ) {
            Some((off, conf)) => {
                tracing::info!(
                    "Seed offset vote: offset={off} confidence={:.0}%",
                    conf * 100.0
                );
                let result = OffsetResult {
                    offset: off,
                    confidence: conf,
                    method: OffsetMethod::SeedBased,
                    evidence: vec![],
                };
                (off, Some(result))
            }
            None => {
                tracing::warn!(
                    "Seed offset vote was inconclusive — trying standard offset detection"
                );
                let fallback = offset::calculate_offset(doc, None, config.toc_pages);
                // If the standard detection is very low-confidence (< 30%), default to
                // offset 0 rather than risking a wildly wrong value that pushes most
                // seeds out of range.
                let (off, result) = match fallback {
                    Some(ref r) if r.confidence >= 0.30 => (r.offset, fallback),
                    _ => {
                        // PageLabels / TOC-text / page-number detection inconclusive.
                        // Last resort: scan ALL text-extractable pages and vote from
                        // the page side (inverse of calculate_offset_from_seeds).
                        tracing::warn!(
                            "Standard offset detection inconclusive — trying page-scan vote"
                        );
                        let scan_threshold = config.fuzzy_threshold.max(0.5);
                        match seed::calculate_offset_by_page_scan(
                            &seeds,
                            doc,
                            scan_threshold,
                        ) {
                            Some((off, conf)) => {
                                tracing::info!(
                                    "Page-scan offset vote: offset={off} confidence={:.0}%",
                                    conf * 100.0
                                );
                                let result = OffsetResult {
                                    offset: off,
                                    confidence: conf,
                                    method: OffsetMethod::SeedBased,
                                    evidence: vec![],
                                };
                                (off, Some(result))
                            }
                            None => {
                                tracing::warn!("No offset detected — using offset 0");
                                let zero = OffsetResult {
                                    offset: 0,
                                    confidence: 0.0,
                                    method: OffsetMethod::SeedBased,
                                    evidence: vec![],
                                };
                                (0, Some(zero))
                            }
                        }
                    }
                };
                (off, result)
            }
        }
    };

    // Phase 3: Resolve seeds → verified page locations.
    let mut resolved = seed::resolve_seeds(
        &seeds,
        offset,
        doc,
        config.fuzzy_threshold,
        config.page_shift_tolerance,
    );

    // Phase 3b: Correct Estimated seeds by nearest-neighbour interpolation
    //           from Confirmed seeds.  Runs before user anchors so that
    //           explicit --anchor values can still override the auto result.
    {
        let total_pages = doc.get_pages().len() as u32;
        seed::correct_estimated_by_confirmed_neighbors(&mut resolved, &seeds, total_pages);
    }

    // Phase 3c: Apply per-segment anchor corrections to Estimated seeds.
    if !config.user_anchors.is_empty() {
        let total_pages = doc.get_pages().len() as u32;
        seed::apply_anchor_corrections(
            &mut resolved,
            &seeds,
            &config.user_anchors,
            total_pages,
        );
        tracing::info!(
            "Applied {} user anchor(s) for per-segment offset correction.",
            config.user_anchors.len()
        );
    }

    let confirmed = resolved
        .iter()
        .filter(|r| r.status == seed::SeedStatus::Confirmed)
        .count();
    let estimated = resolved
        .iter()
        .filter(|r| r.status == seed::SeedStatus::Estimated)
        .count();
    tracing::info!(
        "Seed resolution: {confirmed} confirmed, {estimated} estimated, {} out-of-range",
        resolved.len() - confirmed - estimated
    );

    // Phase 4: Convert resolved seeds → HeadingCandidates.
    let headings = seed::seeds_to_headings(&resolved);

    // Phases 5–6: finish (verify against page text + inject).
    let mut result = finish_pipeline(
        doc,
        path,
        config,
        probe_result,
        None,
        headings,
        seed_offset_result,
    )?;

    // Attach seed verification table to result.
    result.seed_verification = Some(resolved);
    Ok(result)
}

// ---------------------------------------------------------------------------
// Shared pipeline tail (phases 4–6)
// ---------------------------------------------------------------------------

/// Complete phases 4–6 of the pipeline (offset, verify, inject) and return
/// the final [`RecoveryResult`].
///
/// Extracted so that both the TextBased/Mixed path and the seeded path
/// can share the same logic without duplication.
///
/// `override_offset` — when `Some`, skips the normal offset calculation and
/// uses the supplied value directly (used by the seeded path).
fn finish_pipeline(
    doc: &mut Document,
    _path: &str,
    config: &RecoveryConfig,
    probe_result: ProbeResult,
    layout_result: Option<LayoutResult>,
    headings: Vec<HeadingCandidate>,
    override_offset: Option<OffsetResult>,
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
            seed_verification: None,
        });
    }

    // Phase 4: Offset calculation (skip if an override was supplied).
    let offset_result = override_offset
        .or_else(|| offset::calculate_offset(doc, layout_result.as_ref(), config.toc_pages));

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
        seed_verification: None,
    })
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
