//! Seeded outline recovery — load known chapter titles from a reference source
//! and use them to locate headings in a target PDF whose font encoding is broken.
//!
//! # Workflow
//!
//! 1. Call [`load_seeds_from_pdf`] or [`load_seeds_from_json`] to build a
//!    `Vec<SeedEntry>` of ground-truth chapter titles and their 0-based physical
//!    page indices in the *reference* PDF.
//!
//! 2. Call [`calculate_offset_from_seeds`] to find the consensus offset between
//!    the reference pages and the *target* PDF, even when the target has garbled
//!    text.  Returns an `(offset, confidence)` pair.
//!
//! 3. Call [`resolve_seeds`] to locate
//!    each chapter in the target PDF and produce a `Vec<ResolvedSeed>`.
//!
//! 4. Call [`seeds_to_headings`] to convert the resolved seeds into
//!    `HeadingCandidate` values that feed directly into `finish_pipeline`.

use std::path::Path;

use anyhow::{Context, Result};
use lopdf::Document;
use serde::{Deserialize, Serialize};

use super::heuristics::HeadingCandidate;
use super::outlines::extract_chapters_with_depth_and_level;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single chapter entry supplied as ground truth (from a reference PDF or
/// a manually written JSON file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedEntry {
    /// Chapter title exactly as it appears in the reference source.
    pub title: String,
    /// 0-based physical page index in the *reference* PDF.
    pub ref_page: u32,
    /// Heading depth: 1 = top-level chapter, 2 = section, etc.
    pub depth_level: u32,
}

/// A seed entry after the consensus offset has been applied and the target
/// page has been searched (or estimated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSeed {
    /// The ground-truth title (from the seed, not from garbled page text).
    pub title: String,
    /// Best-estimate 0-based physical page index in the *target* PDF.
    pub target_page: u32,
    /// Heading depth carried from the seed.
    pub depth_level: u32,
    /// How this entry was placed.
    pub status: SeedStatus,
    /// Best similarity score achieved during verification (0.0 if unverified).
    pub similarity: f64,
}

/// Outcome of trying to locate a seed title in the target PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedStatus {
    /// Text search confirmed the title on this page (similarity ≥ threshold).
    Confirmed,
    /// No text match was found; the page was estimated by applying the consensus
    /// offset to the reference page number.
    Estimated,
    /// The computed target page is outside the document's page range.
    OutOfRange,
}

// ---------------------------------------------------------------------------
// JSON seed file schema
// ---------------------------------------------------------------------------

/// On-disk representation of a single seed entry (from a hand-written JSON file).
///
/// The `page` field is the **1-based logical page number** as printed in the book
/// (matching `arcane outline` display).  `depth` is optional (defaults to 1).
#[derive(Debug, Deserialize)]
struct JsonSeedEntry {
    title: String,
    page: u32,
    #[serde(default = "default_depth")]
    depth: u32,
}

fn default_depth() -> u32 {
    1
}

// ---------------------------------------------------------------------------
// Seed loading
// ---------------------------------------------------------------------------

/// Load seed entries from a JSON file.
///
/// The file must contain a JSON array of objects with at least `"title"` and
/// `"page"` fields.  `"page"` is the 1-based logical page number (matching the
/// `arcane outline` display).  `"depth"` is optional (defaults to 1).
///
/// ```json
/// [
///   {"title": "1 Introduction", "page": 21},
///   {"title": "2 Representation...", "page": 35, "depth": 1}
/// ]
/// ```
pub fn load_seeds_from_json(path: &Path) -> Result<Vec<SeedEntry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read seed file {}", path.display()))?;
    let raw: Vec<JsonSeedEntry> = serde_json::from_str(&text)
        .with_context(|| format!("invalid JSON in seed file {}", path.display()))?;
    let mut entries: Vec<SeedEntry> = raw
        .into_iter()
        .map(|e| SeedEntry {
            title: e.title,
            ref_page: e.page.saturating_sub(1), // convert 1-based → 0-based
            depth_level: e.depth.max(1),
        })
        .collect();
    // Deduplicate: keep the first occurrence of each (title, ref_page) pair.
    // Duplicate entries cause repeated bookmark entries in the output PDF.
    let original_len = entries.len();
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert((e.title.clone(), e.ref_page)));
    let removed = original_len - entries.len();
    if removed > 0 {
        tracing::warn!("Removed {removed} duplicate seed entries from JSON file.");
    }
    Ok(entries)
}

/// Load seed entries from a reference PDF's `/Outlines` tree.
///
/// Opens `ref_path` independently of the target document and walks its outline
/// tree up to `max_depth` levels.  The reference PDF **must** have a working
/// `/Outlines` tree (verify with `arcane outline`).
pub fn load_seeds_from_pdf(ref_path: &Path, max_depth: u32) -> Result<Vec<SeedEntry>> {
    let doc = Document::load(ref_path)
        .with_context(|| format!("cannot open reference PDF {}", ref_path.display()))?;
    let entries = extract_chapters_with_depth_and_level(&doc, max_depth).with_context(|| {
        format!(
            "reference PDF has no usable /Outlines: {}",
            ref_path.display()
        )
    })?;
    Ok(entries
        .into_iter()
        .map(|(ref_page, title, depth_level)| SeedEntry {
            title,
            ref_page,
            depth_level,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Offset detection from seeds
// ---------------------------------------------------------------------------

/// Vote over candidate offsets to find the consensus page delta between
/// the reference PDF and the target PDF.
///
/// Returns `Some((offset, confidence))` when at least 2 seeds agree on a
/// single delta, or `None` when the vote is inconclusive.
///
/// # Algorithm
///
/// For each candidate offset `d` in `[-tolerance, +tolerance]`:
///   - For every seed, compute `target_page = ref_page as i32 + d`.
///   - Extract that page's text from the target document.
///   - Compute fuzzy similarity between the seed title and each line of the text.
///   - If the best line similarity ≥ `fuzzy_threshold`, the seed *votes* for `d`.
///
///     The offset with the most votes wins.
pub fn calculate_offset_from_seeds(
    seeds: &[SeedEntry],
    doc: &Document,
    fuzzy_threshold: f64,
    tolerance: u32,
) -> Option<(i32, f32)> {
    let total_pages = doc.get_pages().len() as i32;
    let tol = tolerance as i32;

    // votes[d + tol] = number of seeds that matched at offset d.
    let width = (tol * 2 + 1) as usize;
    let mut votes = vec![0usize; width];

    for seed in seeds {
        for d in -tol..=tol {
            let target_page = seed.ref_page as i32 + d;
            if target_page < 0 || target_page >= total_pages {
                continue;
            }
            // lopdf page numbers are 1-based.
            let lopdf_page = (target_page + 1) as u32;
            let text = match doc.extract_text(&[lopdf_page]) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let sim = best_line_similarity(&seed.title, &text);
            if sim >= fuzzy_threshold {
                let idx = (d + tol) as usize;
                votes[idx] += 1;
            }
        }
    }

    // Find the winning offset.
    let (best_idx, &best_votes) = votes.iter().enumerate().max_by_key(|(_, v)| *v)?;

    if best_votes < 2 {
        return None; // inconclusive
    }

    let best_offset = best_idx as i32 - tol;
    let confidence = (best_votes as f32 / seeds.len() as f32).min(1.0) * 0.95;
    Some((best_offset, confidence))
}

// ---------------------------------------------------------------------------
// Offset detection by inverse page scan
// ---------------------------------------------------------------------------

/// Offset detection by scanning all text-extractable pages (last-resort fallback).
///
/// Unlike [`calculate_offset_from_seeds`], which searches from seed → page,
/// this function searches from page → seed: for every page with non-empty
/// extracted text it tries every seed and computes the implied candidate offset
///
/// ```text
/// candidate_offset = physical_0based - seed.ref_page
/// ```
///
/// The most-voted offset wins.  Especially useful for Mixed (scanned + text)
/// PDFs where only a few structural pages (e.g. Part headers) are readable —
/// those pages vote for the correct offset even though chapter pages are
/// scanned and chapter seeds can't be found by the forward seed-based vote.
///
/// Uses a stricter `fuzzy_threshold` (typically ≥ 0.5) than the seed-vote
/// (0.15) to suppress false positives from short or common-word titles.
///
/// Returns `Some((offset, confidence))` when at least 2 independent pages
/// agree on the same offset, otherwise `None`.
pub fn calculate_offset_by_page_scan(
    seeds: &[SeedEntry],
    doc: &Document,
    fuzzy_threshold: f64,
) -> Option<(i32, f32)> {
    use std::collections::HashMap;

    let total_pages = doc.get_pages().len() as u32;
    let mut offset_votes: HashMap<i32, u32> = HashMap::new();

    for phys_0based in 0..total_pages {
        let text = match doc.extract_text(&[phys_0based + 1]) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => continue,
        };

        for seed in seeds {
            let sim = best_line_similarity(&seed.title, &text);
            if sim >= fuzzy_threshold {
                // offset = physical_0based - logical_0based (= seed.ref_page)
                let candidate_offset = phys_0based as i32 - seed.ref_page as i32;
                *offset_votes.entry(candidate_offset).or_insert(0) += 1;
            }
        }
    }

    let (best_offset, best_votes) = offset_votes.into_iter().max_by_key(|(_, v)| *v)?;
    if best_votes < 2 {
        return None; // inconclusive
    }

    let confidence = (best_votes as f32 / seeds.len() as f32).min(1.0) * 0.75;
    Some((best_offset, confidence))
}

// ---------------------------------------------------------------------------
// Seed resolution (text-based, always compiled)
// ---------------------------------------------------------------------------

/// Locate each seed title in the target document using `lopdf` text extraction.
///
/// For each seed, applies `offset` to compute the expected target page, then
/// searches within `±tolerance` pages for the best text match.  Sets
/// `status = Confirmed` when similarity ≥ `fuzzy_threshold`, otherwise
/// `Estimated`.
///
/// This path is always available (no feature gate).  For PDFs with broken
/// font encoding the extracted text will often be garbled, so most seeds will
/// come back `Estimated` — but the *titles* are still correct (they come from
/// the seed, not from the garbled page).
pub fn resolve_seeds(
    seeds: &[SeedEntry],
    offset: i32,
    doc: &Document,
    fuzzy_threshold: f64,
    tolerance: u32,
) -> Vec<ResolvedSeed> {
    let total_pages = doc.get_pages().len() as i32;
    let tol = tolerance as i32;

    seeds
        .iter()
        .map(|seed| {
            let base = seed.ref_page as i32 + offset;
            if base < 0 || base >= total_pages {
                return ResolvedSeed {
                    title: seed.title.clone(),
                    target_page: base.max(0) as u32,
                    depth_level: seed.depth_level,
                    status: SeedStatus::OutOfRange,
                    similarity: 0.0,
                };
            }

            // Search ±tolerance pages for the best match.
            let mut best_page = base as u32;
            let mut best_sim = 0.0f64;

            for d in -tol..=tol {
                let p = base + d;
                if p < 0 || p >= total_pages {
                    continue;
                }
                let lopdf_page = (p + 1) as u32;
                let text = match doc.extract_text(&[lopdf_page]) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let sim = best_line_similarity(&seed.title, &text);
                if sim > best_sim {
                    best_sim = sim;
                    best_page = p as u32;
                }
            }

            if best_sim >= fuzzy_threshold {
                return ResolvedSeed {
                    title: seed.title.clone(),
                    target_page: best_page,
                    depth_level: seed.depth_level,
                    status: SeedStatus::Confirmed,
                    similarity: best_sim,
                };
            }

            // Title fuzzy-match failed (garbled text or scanned PDF).
            // Fallback: search for a page whose printed header/footer number
            // matches the seed's logical page number.  This handles variable
            // blank-page offsets that the title search misses.
            let logical_page = seed.ref_page + 1; // ref_page = logical_page - 1 for JSON seeds
            for d in -tol..=tol {
                let candidate = base + d;
                if candidate < 0 || candidate >= total_pages {
                    continue;
                }
                if extract_page_number(doc, candidate as u32) == Some(logical_page) {
                    return ResolvedSeed {
                        title: seed.title.clone(),
                        target_page: candidate as u32,
                        depth_level: seed.depth_level,
                        status: SeedStatus::Confirmed,
                        similarity: 1.0,
                    };
                }
            }

            // Neither title nor page-number matched — fall back to the
            // un-shifted base page as the best estimate.
            ResolvedSeed {
                title: seed.title.clone(),
                target_page: base as u32,
                depth_level: seed.depth_level,
                status: SeedStatus::Estimated,
                similarity: best_sim,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-segment anchor correction
// ---------------------------------------------------------------------------

/// Re-target `Estimated` seeds using user-supplied per-segment anchor points.
///
/// Each anchor is a `(logical_1based, physical_1based)` pair that pins a
/// known-correct logical→physical mapping.  For every Estimated seed the
/// function finds the nearest anchor whose logical page is ≤ the seed's
/// logical page and re-computes the target using that anchor's local offset:
///
/// ```text
/// local_offset = physical - logical          (both 1-based, so units match)
/// new_target_0based = seed.ref_page + local_offset
/// ```
///
/// Confirmed seeds are never modified.  Anchors that produce out-of-range
/// results are silently skipped.
///
/// # Note on parallelism
/// `resolved` and `seeds` must be in the same order (i.e. `resolved[i]`
/// corresponds to `seeds[i]`), which is guaranteed when both come from a
/// single `resolve_seeds()` call on the same `seeds` slice.
pub fn apply_anchor_corrections(
    resolved: &mut [ResolvedSeed],
    seeds: &[SeedEntry],
    user_anchors: &[(u32, u32)],
    total_pages: u32,
) {
    if user_anchors.is_empty() {
        return;
    }

    // Build sorted (ref_page_0based, local_offset) pairs.
    let mut anchors: Vec<(u32, i32)> = user_anchors
        .iter()
        .map(|&(logical, physical)| {
            let ref_page = logical - 1;
            let local_offset = physical as i32 - 1 - ref_page as i32;
            (ref_page, local_offset)
        })
        .collect();
    anchors.sort_by_key(|&(rp, _)| rp);

    for (rs, seed) in resolved.iter_mut().zip(seeds.iter()) {
        if rs.status != SeedStatus::Estimated {
            continue;
        }
        // Find the nearest anchor whose ref_page ≤ this seed's ref_page.
        let local_off = anchors
            .iter()
            .take_while(|&&(ap, _)| ap <= seed.ref_page)
            .last()
            .map(|&(_, off)| off);

        if let Some(off) = local_off {
            let new_target = seed.ref_page as i32 + off;
            if new_target >= 0 && (new_target as u32) < total_pages {
                rs.target_page = new_target as u32;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Confirmed-neighbour interpolation for Estimated seeds
// ---------------------------------------------------------------------------

/// Re-target `Estimated` seeds by interpolating from the nearest `Confirmed`
/// neighbour (by logical page distance).
///
/// For each Estimated seed, the function finds the confirmed seed whose
/// `ref_page` is closest (ties broken in favour of the predecessor).  It
/// then applies that confirmed seed's local offset:
///
/// ```text
/// local_offset = target_page - ref_page   (both 0-based)
/// new_target   = seed.ref_page + local_offset
/// ```
///
/// Confirmed seeds are never modified.  Results that fall out of the
/// document page range are silently skipped (the seed keeps its previous
/// estimate).
///
/// Call this **before** [`apply_anchor_corrections`] so that explicit
/// user anchors can override the automatic correction.
pub fn correct_estimated_by_confirmed_neighbors(
    resolved: &mut [ResolvedSeed],
    seeds: &[SeedEntry],
    total_pages: u32,
) {
    let n = resolved.len();

    // Pre-compute the local offset for every confirmed seed (None for non-confirmed).
    // offset[i] = Some(target_page - ref_page) when resolved[i] is Confirmed.
    let offsets: Vec<Option<(u32, i32)>> = resolved
        .iter()
        .zip(seeds.iter())
        .map(|(rs, seed)| {
            if rs.status == SeedStatus::Confirmed {
                Some((seed.ref_page, rs.target_page as i32 - seed.ref_page as i32))
            } else {
                None
            }
        })
        .collect();

    let any_confirmed = offsets.iter().any(|o| o.is_some());
    if !any_confirmed {
        return;
    }

    let mut corrections = 0u32;
    for i in 0..n {
        if resolved[i].status != SeedStatus::Estimated {
            continue;
        }
        let seed_ref = seeds[i].ref_page;

        // Nearest confirmed BEFORE this seed in document order.
        let before_opt = (0..i).rev().find_map(|j| offsets[j]);

        // Nearest confirmed AFTER this seed in document order.
        let after_opt = (i + 1..n).find_map(|j| offsets[j]);

        let local_off = match (before_opt, after_opt) {
            (Some((brp, boff)), Some((arp, aoff))) => {
                // Prefer the ref_page-closer neighbour; ties go to the predecessor.
                // Use signed distance to handle rare out-of-order ref_page cases.
                let dist_before = (seed_ref as i64 - brp as i64).abs();
                let dist_after = (arp as i64 - seed_ref as i64).abs();
                if dist_before <= dist_after {
                    boff
                } else {
                    aoff
                }
            }
            (Some((_, off)), None) => off,
            (None, Some((_, off))) => off,
            (None, None) => continue,
        };

        let new_target = seed_ref as i32 + local_off;
        if new_target >= 0 && (new_target as u32) < total_pages {
            resolved[i].target_page = new_target as u32;
            corrections += 1;
        }
    }
    if corrections > 0 {
        tracing::info!("Neighbour interpolation corrected {corrections} Estimated seed(s).");
    }
}

// ---------------------------------------------------------------------------
// Convert resolved seeds → HeadingCandidates
// ---------------------------------------------------------------------------

/// Convert resolved seeds into [`HeadingCandidate`] values for injection.
///
/// `OutOfRange` entries are silently skipped.  Font sizes are cosmetic
/// (depth 1 = 18 pt, depth 2 = 14 pt, depth ≥ 3 = 12 pt).
pub fn seeds_to_headings(resolved: &[ResolvedSeed]) -> Vec<HeadingCandidate> {
    resolved
        .iter()
        .filter(|r| r.status != SeedStatus::OutOfRange)
        .map(|r| {
            let font_size = match r.depth_level {
                1 => 18.0f32,
                2 => 14.0,
                _ => 12.0,
            };
            HeadingCandidate {
                page_index: r.target_page,
                font_size,
                text: r.title.clone(),
                y_position: None,
                depth_level: r.depth_level,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Attempt to extract the printed logical page number from a PDF page.
///
/// Reads the first and last few non-empty lines of the extracted text and
/// returns the first standalone integer found in those lines.  Most academic
/// books place the page number in the header or footer, so this heuristic
/// works well for text-based PDFs.
///
/// Returns `None` if text extraction fails or no plausible page number is
/// found.
fn extract_page_number(doc: &Document, physical_page_0based: u32) -> Option<u32> {
    let text = doc.extract_text(&[physical_page_0based + 1]).ok()?;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let n = lines.len();
    // Check header (first 3 lines) and footer (last 3 lines), deduplicating
    // for short pages.
    let mut seen = std::collections::BTreeSet::new();
    let indices: Vec<usize> = (0..3.min(n))
        .chain(n.saturating_sub(3)..n)
        .filter(|i| seen.insert(*i))
        .collect();
    for i in indices {
        for tok in lines[i].split_whitespace() {
            // Strip any non-digit prefix/suffix (e.g. "–42–" → "42").
            let digits: String = tok.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                continue;
            }
            if let Ok(num) = digits.parse::<u32>() {
                if num > 0 && num < 10_000 {
                    return Some(num);
                }
            }
        }
    }
    None
}

/// Return the best normalised-Levenshtein similarity between `title` and any
/// non-empty line in `text`.  Both sides are lower-cased before comparison.
fn best_line_similarity(title: &str, text: &str) -> f64 {
    let needle = title.to_lowercase();
    text.lines()
        .map(|line| {
            let haystack = line.trim().to_lowercase();
            if haystack.is_empty() {
                0.0
            } else {
                strsim::normalized_levenshtein(&needle, &haystack)
            }
        })
        .fold(0.0f64, f64::max)
}
