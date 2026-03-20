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
    Ok(raw
        .into_iter()
        .map(|e| SeedEntry {
            title: e.title,
            ref_page: e.page.saturating_sub(1), // convert 1-based → 0-based
            depth_level: e.depth.max(1),
        })
        .collect())
}

/// Load seed entries from a reference PDF's `/Outlines` tree.
///
/// Opens `ref_path` independently of the target document and walks its outline
/// tree up to `max_depth` levels.  The reference PDF **must** have a working
/// `/Outlines` tree (verify with `arcane outline`).
pub fn load_seeds_from_pdf(ref_path: &Path, max_depth: u32) -> Result<Vec<SeedEntry>> {
    let doc = Document::load(ref_path)
        .with_context(|| format!("cannot open reference PDF {}", ref_path.display()))?;
    let entries = extract_chapters_with_depth_and_level(&doc, max_depth)
        .with_context(|| format!("reference PDF has no usable /Outlines: {}", ref_path.display()))?;
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
/// The offset with the most votes wins.
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
    let (best_idx, &best_votes) = votes
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| *v)?;

    if best_votes < 2 {
        return None; // inconclusive
    }

    let best_offset = best_idx as i32 - tol;
    let confidence = (best_votes as f32 / seeds.len() as f32).min(1.0) * 0.95;
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

            let status = if best_sim >= fuzzy_threshold {
                SeedStatus::Confirmed
            } else {
                SeedStatus::Estimated
            };

            // For Estimated entries always use the un-shifted base page
            // (most reliable when text is garbled).
            let target_page = if status == SeedStatus::Estimated {
                base as u32
            } else {
                best_page
            };

            ResolvedSeed {
                title: seed.title.clone(),
                target_page,
                depth_level: seed.depth_level,
                status,
                similarity: best_sim,
            }
        })
        .collect()
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
