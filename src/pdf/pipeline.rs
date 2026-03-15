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
// OCR pipeline config (feature-gated)
// ---------------------------------------------------------------------------

/// Configuration specific to the OCR-only pipeline (`--ocr` mode).
#[cfg(feature = "ocr")]
pub struct OcrPipelineConfig {
    /// Render DPI for OCR (default 150).
    pub dpi: u32,
    /// Language hint (currently unused, reserved for future multi-lang).
    #[allow(dead_code)]
    pub lang: String,
    /// Model variant (currently unused, reserved for server model selection).
    #[allow(dead_code)]
    pub model: Option<String>,
    /// Manual page offset override (`--page-offset`).
    pub manual_offset: Option<i32>,
    /// Emit debug layout JSON to stderr (`--debug-layout`).
    pub debug_layout: bool,
}

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
                        return finish_pipeline(doc, path, config, probe_result, None, h, None);
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
                    seed_verification: None,
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
                    seed_verification: None,
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
            seed_verification: None,
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
        None,
    )
}

// ---------------------------------------------------------------------------
// Seeded pipeline entry point
// ---------------------------------------------------------------------------

/// Run the seeded outline recovery pipeline.
///
/// Uses `seeds` (from `--seed-pdf` or `--seed-file`) as ground-truth chapter
/// titles, bypassing heuristic/OCR heading *detection*.  OCR or text search
/// is used only to *verify* which physical page each seed title lands on.
///
/// Flow:
/// 1. **Probe** — classify document type.
/// 2. **Offset from seeds** — vote over candidate offsets; fall back to
///    standard detection if the vote is inconclusive.
/// 3. **Resolve** — locate each seed in the target PDF (OCR when available,
///    otherwise lopdf text extraction).
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
    let _total_pages = probe_result.total_pages;
    #[cfg(feature = "ocr")]
    let total_pages = _total_pages;

    // Phase 2: Offset from seeds.
    // Use a lower internal threshold (0.15) for the vote so that even partially-
    // readable text produces a signal.  The full fuzzy_threshold is reserved for
    // seed *verification* (resolve_seeds / verify_seeds_ocr).
    let vote_threshold = config.fuzzy_threshold.min(0.15);
    let (offset, seed_offset_result) = match seed::calculate_offset_from_seeds(
        &seeds,
        doc,
        vote_threshold,
        config.page_shift_tolerance,
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
                Some(ref r) => {
                    tracing::warn!(
                        "Standard offset detection low-confidence ({:.0}%) — using offset 0",
                        r.confidence * 100.0
                    );
                    let zero = OffsetResult {
                        offset: 0,
                        confidence: 0.0,
                        method: OffsetMethod::SeedBased,
                        evidence: vec![],
                    };
                    (0, Some(zero))
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
            };
            (off, result)
        }
    };

    // Phase 3: Resolve seeds → verified page locations.
    #[cfg(feature = "ocr")]
    let resolved = seed::verify_seeds_ocr(
        &seeds,
        offset,
        std::path::Path::new(path),
        config.fuzzy_threshold,
        config.page_shift_tolerance,
        total_pages,
    )
    .unwrap_or_else(|e| {
        tracing::warn!("Seed OCR verification failed ({e:#}), falling back to text extraction");
        seed::resolve_seeds(&seeds, offset, doc, config.fuzzy_threshold, config.page_shift_tolerance)
    });

    #[cfg(not(feature = "ocr"))]
    let resolved = seed::resolve_seeds(
        &seeds,
        offset,
        doc,
        config.fuzzy_threshold,
        config.page_shift_tolerance,
    );

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
/// Extracted so that both the TextBased/Mixed path and the Scanned/OCR path
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
// OCR-only pipeline (feature-gated)
// ---------------------------------------------------------------------------

/// Run the OCR-only TOC reconstruction pipeline.
///
/// Bypasses all font-histogram heuristics. Reads the TOC pages via OCR,
/// parses structured entries, reconstructs hierarchy, estimates offset,
/// and injects hierarchical outlines.
///
/// Called when `--ocr` is specified on the `recover-outline` command.
#[cfg(feature = "ocr")]
pub fn recover_outline_ocr(
    doc: &mut Document,
    path: &str,
    config: &RecoveryConfig,
    ocr_config: &OcrPipelineConfig,
) -> Result<RecoveryResult> {
    use super::{ocr, ocr_ir, toc_extract, toc_hierarchy};

    // Phase 1: Probe.
    let probe_result = probe::probe(doc, path);
    let total_pages = probe_result.total_pages;

    // Phase 2: Determine TOC page range.
    let toc_range = config.toc_pages.unwrap_or_else(|| {
        let end = (total_pages.saturating_sub(1)).min(14);
        (0, end)
    });
    let toc_page_indices: Vec<u32> = (toc_range.0..=toc_range.1).collect();

    tracing::info!(
        "OCR pipeline: scanning TOC pages {}-{} ({} pages) at {} DPI",
        toc_range.0,
        toc_range.1,
        toc_page_indices.len(),
        ocr_config.dpi
    );

    // Phase 3: OCR the TOC pages.
    let ocr_results = ocr::extract_text_ocr(
        std::path::Path::new(path),
        &toc_page_indices,
        ocr_config.dpi,
    )
    .context("OCR failed on TOC pages")?;

    // Phase 4: Get page dimensions from lopdf.
    let page_dims = get_page_dimensions(doc, &toc_page_indices);

    // Phase 5: Normalize OCR → IR.
    let ocr_pages = ocr_ir::normalize_pages(&ocr_results, &page_dims);

    // Debug output.
    if ocr_config.debug_layout {
        if let Ok(debug_json) = serde_json::to_string_pretty(&ocr_pages) {
            eprintln!("[debug-layout]\n{debug_json}");
        }
    }

    // Phase 6: Extract TOC entries.
    let toc_entries = toc_extract::extract_toc_entries(&ocr_pages);

    if toc_entries.is_empty() {
        tracing::warn!(
            "OCR pipeline: no TOC entries found on pages {}-{}",
            toc_range.0,
            toc_range.1
        );
        return Ok(RecoveryResult {
            probe: probe_result,
            layout: None,
            offset: None,
            headings: vec![],
            chapter_map: std::collections::BTreeMap::new(),
            verification: vec![],
            injected_count: None,
            seed_verification: None,
        });
    }

    tracing::info!("OCR pipeline: extracted {} TOC entries", toc_entries.len());

    // Phase 7: Assign hierarchy.
    let hierarchy_config = toc_hierarchy::HierarchyConfig {
        max_depth: config.max_depth,
        ..toc_hierarchy::HierarchyConfig::default()
    };
    let hierarchical = toc_hierarchy::assign_hierarchy(&toc_entries, &hierarchy_config);

    // Phase 8: Estimate page offset.
    let offset_result =
        toc_extract::estimate_offset_from_toc(&toc_entries, doc, ocr_config.manual_offset);

    for warning in &offset_result.warnings {
        tracing::warn!("OCR offset: {warning}");
    }

    let active_offset = ocr_config
        .manual_offset
        .unwrap_or(offset_result.arabic_offset);

    tracing::info!(
        "OCR pipeline: using offset {:+} (confidence: {:.0}%)",
        active_offset,
        offset_result.confidence * 100.0
    );

    // Phase 9: Convert to HeadingCandidates with physical page indices.
    let headings: Vec<HeadingCandidate> = hierarchical
        .iter()
        .filter_map(|h| {
            let page_num = h.entry.page_number?;
            let physical = (page_num as i32 - 1 + active_offset) as i64;
            if physical < 0 || physical >= total_pages as i64 {
                return None;
            }
            Some(HeadingCandidate {
                page_index: physical as u32,
                font_size: match h.depth {
                    1 => 18.0,
                    2 => 14.0,
                    _ => 12.0,
                },
                text: h.entry.title.clone(),
                y_position: None,
                depth_level: h.depth,
            })
        })
        .collect();

    if headings.is_empty() {
        tracing::warn!("OCR pipeline: no headings after page-offset mapping");
        return Ok(RecoveryResult {
            probe: probe_result,
            layout: None,
            offset: None,
            headings: vec![],
            chapter_map: std::collections::BTreeMap::new(),
            verification: vec![],
            injected_count: None,
            seed_verification: None,
        });
    }

    tracing::info!(
        "OCR pipeline: {} heading(s) mapped to physical pages",
        headings.len()
    );

    // Phase 10: Finish pipeline (verify + inject).
    let pipeline_offset = Some(OffsetResult {
        offset: active_offset as i32,
        confidence: offset_result.confidence,
        method: OffsetMethod::OcrTocParsing,
        evidence: vec![],
    });

    finish_pipeline(
        doc,
        path,
        config,
        probe_result,
        None,
        headings,
        pipeline_offset,
    )
}

/// Get page dimensions from the lopdf Document for specified page indices.
///
/// Parses the `/MediaBox` from each page dictionary, defaulting to US Letter
/// (612×792 points) if not found.
#[cfg(feature = "ocr")]
fn get_page_dimensions(doc: &Document, page_indices: &[u32]) -> Vec<(f32, f32)> {
    use lopdf::Object;

    let pages = doc.get_pages();

    page_indices
        .iter()
        .map(|&idx| {
            let page_num = idx + 1; // lopdf uses 1-based
            if let Some(&page_oid) = pages.get(&page_num) {
                if let Ok(page_dict) = doc.get_dictionary(page_oid) {
                    if let Ok(Object::Array(media_box)) = page_dict.get(b"MediaBox") {
                        if media_box.len() == 4 {
                            let w = obj_to_f32(&media_box[2]).unwrap_or(612.0);
                            let h = obj_to_f32(&media_box[3]).unwrap_or(792.0);
                            return (w, h);
                        }
                    }
                }
            }
            (612.0, 792.0) // US Letter default
        })
        .collect()
}

/// Extract an f32 from a lopdf Object (Integer or Real).
#[cfg(feature = "ocr")]
fn obj_to_f32(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Integer(i) => Some(*i as f32),
        lopdf::Object::Real(r) => Some(*r as f32),
        _ => None,
    }
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
