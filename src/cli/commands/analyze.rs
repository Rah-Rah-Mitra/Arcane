//! PDF analysis and inspection commands.
//!
//! These commands are the analytical building blocks used by recovery and
//! chunking workflows.  Each wraps a single `crate::pdf` analysis function
//! with structured or human-readable output.
//!
//! # Analysis operations
//!
//! | Command        | Underlying function                             |
//! |----------------|-------------------------------------------------|
//! | `probe`        | `pdf::probe::probe`                             |
//! | `outline`      | `pdf::outlines::extract_chapters_with_depth`    |
//! | `layout`       | `pdf::layout::analyze_layout`                  |
//! | `offset`       | `pdf::offset::calculate_offset`                |
//! | `sync-pages`   | RANSAC heading↔TOC consensus (inline)           |

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::helpers::parse_page_range;

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

pub fn cmd_probe(file: std::path::PathBuf, json: bool) -> Result<()> {
    use crate::pdf::probe;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    let result = probe::probe(&doc, &file.display().to_string());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("failed to serialise probe result")?
        );
        return Ok(());
    }

    // Human-readable output.
    println!("File:         {}", result.path);
    println!("Pages:        {}", result.total_pages);
    println!("Type:         {}", result.document_kind);
    println!("Text pages:   {}", result.text_page_count);
    println!("Image pages:  {}", result.image_page_count);
    println!(
        "Has outlines: {}",
        if result.has_outlines { "yes" } else { "no" }
    );
    println!(
        "Page labels:  {}",
        if result.has_page_labels { "yes" } else { "no" }
    );

    if result.total_pages <= 30 {
        println!("\nPer-page breakdown:");
        for (i, kind) in result.page_kinds.iter().enumerate() {
            println!("  page {:>4}  {}", i + 1, kind);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Outline display
// ---------------------------------------------------------------------------

pub fn cmd_outline(path: PathBuf, depth: u32) -> Result<()> {
    use crate::pdf::outlines::extract_chapters_with_depth;
    use crate::pdf::page_labels::extract_chapters_from_page_labels;

    let doc = lopdf::Document::load(&path)
        .with_context(|| format!("failed to open PDF at {}", path.display()))?;

    let total_pages = doc.get_pages().len();
    println!("File: {}", path.display());
    println!("Total pages: {total_pages}\n");

    // ── Outlines ─────────────────────────────────────────────────────────
    println!("Outlines (depth {depth}):");
    match extract_chapters_with_depth(&doc, depth) {
        Ok(chapters) => {
            let total = doc.get_pages().len() as u32;
            let ranges = crate::pdf::engine::boundaries_to_ranges(&chapters, total);
            println!("  {:<4} {:<50} Pages", "#", "Title");
            println!("  {}", "\u{2500}".repeat(70));
            for (i, (start, end, title)) in ranges.iter().enumerate() {
                println!(
                    "  {:<4} {:<50} {}-{}",
                    format!("{:02}", i + 1),
                    title,
                    start + 1,
                    end + 1
                );
            }
        }
        Err(e) => println!("  (none — {e})"),
    }

    // ── Page labels ──────────────────────────────────────────────────────
    println!("\nPage labels:");
    match extract_chapters_from_page_labels(&doc) {
        Ok(labels) => {
            for (page_idx, label) in &labels {
                println!("  Page {} — {}", page_idx + 1, label);
            }
        }
        Err(e) => println!("  (none — {e})"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Layout analysis
// ---------------------------------------------------------------------------

pub fn cmd_detect_layout(
    file: std::path::PathBuf,
    json: bool,
    _pages: Option<String>,
) -> Result<()> {
    use crate::pdf::layout;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    let result = layout::analyze_layout(&doc, &file.display().to_string());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("failed to serialise layout result")?
        );
        return Ok(());
    }

    // Human-readable summary.
    println!("File:           {}", result.path);
    println!("Pages:          {}", result.total_pages);
    println!("Body font size: {:.1}pt", result.body_font_size);
    println!("\nFont clusters:");
    for fc in &result.font_clusters {
        println!(
            "  {:.1}pt  {:>8} chars  {:?}",
            fc.centroid, fc.char_count, fc.role
        );
    }

    if result.anchors.is_empty() {
        println!("\nNo structural anchors detected.");
    } else {
        println!("\nStructural anchors ({}):", result.anchors.len());
        for a in &result.anchors {
            println!(
                "  page {:>4}  y={:>6.1}  {:<18}  {:.0}  {}",
                a.page_index + 1,
                a.y,
                format!("{:?}", a.kind),
                a.font_size,
                a.text
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Page offset calculation
// ---------------------------------------------------------------------------

pub fn cmd_find_offset(
    file: std::path::PathBuf,
    toc_pages: Option<String>,
    json: bool,
) -> Result<()> {
    use crate::pdf::offset;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    // Parse optional --toc-pages "start-end" (1-based) → 0-based inclusive range.
    let toc_range = toc_pages.as_deref().and_then(parse_page_range);

    let result = offset::calculate_offset(&doc, None, toc_range);

    match result {
        Some(ref r) if json => {
            println!(
                "{}",
                serde_json::to_string_pretty(r).context("failed to serialise offset result")?
            );
        }
        Some(ref r) => {
            println!("File:       {}", file.display());
            println!("Offset:     {:+}", r.offset);
            println!("Confidence: {:.0}%", r.confidence * 100.0);
            println!("Method:     {:?}", r.method);
            if !r.evidence.is_empty() {
                println!("\nEvidence:");
                for e in &r.evidence {
                    println!(
                        "  physical page {:>4} → printed page {:>4}  ({})",
                        e.physical_page, e.logical_number, e.matched_text
                    );
                }
            }
        }
        None => {
            if json {
                println!("null");
            } else {
                println!(
                    "[arcane] Could not determine page offset for '{}'.",
                    file.display()
                );
                println!("         Try providing --toc-pages <start>-<end> (1-based).");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// sync-pages  (RANSAC heading↔TOC consensus offset)
// ---------------------------------------------------------------------------

/// One matched heading ↔ TOC-entry pair.
#[derive(Debug, serde::Serialize)]
pub struct SyncMatch {
    pub toc_title: String,
    pub printed_page: u32,
    pub physical_page: u32,
    pub similarity: f64,
    pub delta: i32,
    pub is_inlier: bool,
}

/// Result of `sync-pages` consensus offset estimation.
#[derive(Debug, serde::Serialize)]
pub struct PageSyncResult {
    pub consensus_offset: i32,
    /// Fraction of candidates consistent with the consensus offset.
    pub confidence: f32,
    pub total_candidates: usize,
    pub inlier_count: usize,
    pub matches: Vec<SyncMatch>,
}

pub fn cmd_sync_pages(
    file: std::path::PathBuf,
    toc_pages: Option<String>,
    threshold: f64,
    json: bool,
) -> Result<()> {
    use crate::pdf::{layout, offset};

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    // Run full layout analysis (feature-vector pipeline).
    let layout_result = layout::analyze_layout(&doc, &file.display().to_string());

    // Determine TOC pages.
    let toc_page_range: Option<(u32, u32)> = if let Some(ref s) = toc_pages {
        parse_page_range(s)
    } else {
        None
    };

    // Extract positioned text for TOC parsing.
    let positioned = layout::extract_all_positioned(&doc);

    // Detect or use supplied TOC pages.
    let effective_toc_range: Option<(u32, u32)> = if let Some(range) = toc_page_range {
        Some(range)
    } else {
        let auto = layout::detect_toc_pages(&positioned);
        if auto.is_empty() {
            None
        } else {
            let &min_p = auto.iter().min().unwrap();
            let &max_p = auto.iter().max().unwrap();
            Some((min_p, max_p))
        }
    };

    // Parse TOC entries: (title, printed_page).
    let toc_entries: Vec<(String, u32)> = if let Some(range) = effective_toc_range {
        offset::parse_toc_entries(&positioned, range)
    } else {
        vec![]
    };

    if toc_entries.is_empty() {
        if json {
            let result = PageSyncResult {
                consensus_offset: 0,
                confidence: 0.0,
                total_candidates: 0,
                inlier_count: 0,
                matches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("No TOC entries found. Try --toc-pages <range>.");
        }
        return Ok(());
    }

    // Collect chapter/section heading anchors.
    let heading_anchors: Vec<&layout::LayoutAnchor> = layout_result
        .anchors
        .iter()
        .filter(|a| {
            matches!(
                a.kind,
                layout::AnchorKind::ChapterHeading
                    | layout::AnchorKind::SectionHeading
                    | layout::AnchorKind::NumberedHeading
            )
        })
        .collect();

    // RANSAC offset estimation.
    let mut candidates: Vec<SyncMatch> = Vec::new();
    let mut delta_votes: HashMap<i32, f64> = HashMap::new();

    for (toc_title, printed_page) in &toc_entries {
        for anchor in &heading_anchors {
            let sim = strsim::normalized_levenshtein(anchor.text.trim(), toc_title.trim());
            if sim >= threshold {
                let delta = anchor.page_index as i32 - *printed_page as i32;
                *delta_votes.entry(delta).or_insert(0.0) += sim;
                candidates.push(SyncMatch {
                    toc_title: toc_title.clone(),
                    printed_page: *printed_page,
                    physical_page: anchor.page_index,
                    similarity: sim,
                    delta,
                    is_inlier: false,
                });
            }
        }
    }

    if candidates.is_empty() {
        if json {
            let result = PageSyncResult {
                consensus_offset: 0,
                confidence: 0.0,
                total_candidates: 0,
                inlier_count: 0,
                matches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "No heading↔TOC matches above threshold {threshold:.2}. Try a lower --threshold."
            );
        }
        return Ok(());
    }

    // Consensus = delta with the highest total similarity score.
    let consensus_offset = delta_votes
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(&delta, _)| delta)
        .unwrap_or(0);

    // Mark inliers (|delta - consensus| ≤ 1).
    for m in &mut candidates {
        m.is_inlier = (m.delta - consensus_offset).abs() <= 1;
    }
    let inlier_count = candidates.iter().filter(|m| m.is_inlier).count();
    let confidence = inlier_count as f32 / candidates.len() as f32;

    let result = PageSyncResult {
        consensus_offset,
        confidence,
        total_candidates: candidates.len(),
        inlier_count,
        matches: candidates,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    // Human-readable output.
    println!("File:             {}", file.display());
    println!("Consensus offset: {}", result.consensus_offset);
    println!(
        "Confidence:       {:.0}%  ({} / {} inliers)",
        result.confidence * 100.0,
        result.inlier_count,
        result.total_candidates
    );
    println!();
    println!(
        "{:<40}  {:>5}  {:>8}  {:>6}  Inlier",
        "TOC Title", "Print", "Physical", "Sim"
    );
    println!("{}", "-".repeat(78));
    for m in &result.matches {
        println!(
            "{:<40}  {:>5}  {:>8}  {:>5.2}  {}",
            &m.toc_title[..m.toc_title.len().min(40)],
            m.printed_page,
            m.physical_page + 1,
            m.similarity,
            if m.is_inlier { "✓" } else { "✗" }
        );
    }

    Ok(())
}
