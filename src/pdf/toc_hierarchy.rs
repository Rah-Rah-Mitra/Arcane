//! Hierarchy reconstruction for flat TOC entries.
//!
//! Assigns depth levels to a flat list of `TocEntry` values using three
//! independent signals combined by weighted scoring:
//!
//! 1. **Numbering semantics** — `1` → depth 1, `1.2` → depth 2, `A.1.2` → depth 3
//! 2. **Left indent** — cluster `title_x` values into depth levels
//! 3. **Font/box size hints** — cluster `font_size` values (larger → shallower)
//!
//! This module is compiled only when the `ocr` feature is enabled.

use serde::{Deserialize, Serialize};

use super::toc_extract::TocEntry;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A TOC entry with its assigned hierarchy depth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalTocEntry {
    /// The original TOC entry.
    pub entry: TocEntry,
    /// Depth level: 1 = chapter, 2 = section, 3 = subsection.
    pub depth: u32,
}

/// Configuration for hierarchy reconstruction.
pub struct HierarchyConfig {
    /// Maximum depth level to assign.
    pub max_depth: u32,
    /// Weight for numbering-based signal (0.0–1.0).
    pub numbering_weight: f32,
    /// Weight for indent-based signal (0.0–1.0).
    pub indent_weight: f32,
    /// Weight for font-size signal (0.0–1.0).
    pub font_size_weight: f32,
}

impl Default for HierarchyConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            numbering_weight: 0.5,
            indent_weight: 0.3,
            font_size_weight: 0.2,
        }
    }
}

// ---------------------------------------------------------------------------
// Main algorithm
// ---------------------------------------------------------------------------

/// Assign depth levels to a flat list of `TocEntry` values.
///
/// Combines three signals via weighted scoring and applies sanity checks
/// to ensure a valid hierarchy (no depth jumps without parent chain).
pub fn assign_hierarchy(
    entries: &[TocEntry],
    config: &HierarchyConfig,
) -> Vec<HierarchicalTocEntry> {
    if entries.is_empty() {
        return vec![];
    }

    // Build per-signal depth assignments.
    let numbering_depths: Vec<Option<u32>> = entries
        .iter()
        .map(|e| numbering_depth(&e.title))
        .collect();

    let indent_map = indent_clusters(entries, config.max_depth);
    let font_map = font_size_clusters(entries, config.max_depth);

    let mut result: Vec<HierarchicalTocEntry> = Vec::with_capacity(entries.len());

    for (i, entry) in entries.iter().enumerate() {
        let num_depth = numbering_depths[i];
        let indent_depth = indent_depth_for_x(entry.title_x, &indent_map);
        let font_depth = font_depth_for_size(entry.font_size, &font_map);

        // Weighted vote.
        let depth = if let Some(nd) = num_depth {
            // Numbering is the strongest signal — use it directly if available.
            // Blend with others only as tie-breakers.
            let score_num = nd as f32 * config.numbering_weight;
            let score_ind = indent_depth as f32 * config.indent_weight;
            let score_fnt = font_depth as f32 * config.font_size_weight;
            let total_weight =
                config.numbering_weight + config.indent_weight + config.font_size_weight;
            let weighted = (score_num + score_ind + score_fnt) / total_weight;
            // Round to nearest integer, but bias toward numbering.
            let rounded = weighted.round() as u32;
            // If numbering and rounded disagree, prefer numbering.
            if (rounded as i32 - nd as i32).unsigned_abs() <= 1 {
                nd
            } else {
                nd
            }
        } else {
            // No numbering — use indent and font signals only.
            let total_weight = config.indent_weight + config.font_size_weight;
            if total_weight == 0.0 {
                1
            } else {
                let weighted = (indent_depth as f32 * config.indent_weight
                    + font_depth as f32 * config.font_size_weight)
                    / total_weight;
                (weighted.round() as u32).max(1)
            }
        };

        // Clamp to max_depth.
        let depth = depth.min(config.max_depth).max(1);

        result.push(HierarchicalTocEntry {
            entry: entry.clone(),
            depth,
        });
    }

    // Sanity pass: ensure no depth jumps > 1 without parent chain.
    sanitize_depths(&mut result);

    result
}

// ---------------------------------------------------------------------------
// Signal 1: Numbering depth
// ---------------------------------------------------------------------------

/// Parse a section number prefix and return its implied depth.
///
/// - `"1 Introduction"` → Some(1)
/// - `"1.2 Methods"` → Some(2)
/// - `"1.2.3 Sub-method"` → Some(3)
/// - `"A Appendix"` → Some(1)
/// - `"A.1 First section"` → Some(2)
/// - `"A.1.2 Detail"` → Some(3)
/// - `"Bibliography"` → None
pub fn numbering_depth(title: &str) -> Option<u32> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Extract the first token.
    let first_token = trimmed.split_whitespace().next()?;

    // Check if it looks like a section number.
    // Patterns: "1", "1.2", "1.2.3", "A", "A.1", "A.1.2", "3.B"
    let parts: Vec<&str> = first_token.split('.').collect();
    if parts.is_empty() {
        return None;
    }

    // Validate each part is a number or single letter.
    for part in &parts {
        if part.is_empty() {
            return None;
        }
        let is_number = part.chars().all(|c| c.is_ascii_digit());
        let is_letter = part.len() == 1 && part.chars().all(|c| c.is_ascii_alphabetic());
        if !is_number && !is_letter {
            return None;
        }
    }

    Some(parts.len() as u32)
}

// ---------------------------------------------------------------------------
// Signal 2: Indent clusters
// ---------------------------------------------------------------------------

/// Cluster x-positions into indent levels.
///
/// Groups x-values within 5pt into the same cluster, sorts by position,
/// and assigns depth 1, 2, 3, ... to each cluster.
///
/// Returns a sorted list of `(x_threshold, depth)` pairs.
fn indent_clusters(entries: &[TocEntry], max_depth: u32) -> Vec<(f32, u32)> {
    if entries.is_empty() {
        return vec![];
    }

    // Collect unique x-positions (rounded to nearest 5pt).
    let mut x_values: Vec<f32> = entries.iter().map(|e| e.title_x).collect();
    x_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Cluster: group values within 5pt.
    let mut clusters: Vec<f32> = Vec::new();
    for &x in &x_values {
        if clusters
            .last()
            .map_or(true, |&last| (x - last).abs() > 5.0)
        {
            clusters.push(x);
        }
    }

    // Assign depth 1, 2, 3... to each cluster.
    clusters
        .into_iter()
        .enumerate()
        .map(|(i, x)| (x, (i as u32 + 1).min(max_depth)))
        .collect()
}

/// Find the indent depth for a given x-position.
fn indent_depth_for_x(x: f32, clusters: &[(f32, u32)]) -> u32 {
    clusters
        .iter()
        .min_by_key(|(cx, _)| ((cx - x).abs() * 100.0) as u32)
        .map(|(_, depth)| *depth)
        .unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Signal 3: Font size clusters
// ---------------------------------------------------------------------------

/// Cluster font sizes into depth levels (larger = shallower).
///
/// Returns a sorted list of `(font_size_threshold, depth)` pairs.
fn font_size_clusters(entries: &[TocEntry], max_depth: u32) -> Vec<(f32, u32)> {
    if entries.is_empty() {
        return vec![];
    }

    // Collect unique font sizes (rounded to nearest 0.5pt).
    let mut sizes: Vec<f32> = entries
        .iter()
        .map(|e| (e.font_size * 2.0).round() / 2.0)
        .collect();
    sizes.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal)); // descending
    sizes.dedup();

    // Assign depth 1 to largest, 2 to next, etc.
    sizes
        .into_iter()
        .enumerate()
        .map(|(i, sz)| (sz, (i as u32 + 1).min(max_depth)))
        .collect()
}

/// Find the font-size depth for a given size.
fn font_depth_for_size(size: f32, clusters: &[(f32, u32)]) -> u32 {
    let rounded = (size * 2.0).round() / 2.0;
    clusters
        .iter()
        .min_by_key(|(cs, _)| ((cs - rounded).abs() * 100.0) as u32)
        .map(|(_, depth)| *depth)
        .unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Sanity checks
// ---------------------------------------------------------------------------

/// Ensure no depth jumps > 1 without a parent chain.
///
/// If entry i has depth 3 but entry i-1 has depth 1 (jump of 2),
/// promote entry i to depth 2.
fn sanitize_depths(entries: &mut [HierarchicalTocEntry]) {
    if entries.is_empty() {
        return;
    }

    // First entry should be depth 1.
    entries[0].depth = 1.min(entries[0].depth).max(1);

    for i in 1..entries.len() {
        let prev_depth = entries[i - 1].depth;
        let curr_depth = entries[i].depth;

        // Cannot jump more than 1 level deeper.
        if curr_depth > prev_depth + 1 {
            entries[i].depth = prev_depth + 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::toc_extract::TocEntry;

    fn make_entry(title: &str, title_x: f32, font_size: f32) -> TocEntry {
        TocEntry {
            title: title.to_string(),
            page_number_raw: "1".into(),
            page_number: Some(1),
            is_roman: false,
            source_page: 0,
            title_x,
            font_size,
            confidence: 0.9,
        }
    }

    #[test]
    fn test_numbering_depth_parsing() {
        assert_eq!(numbering_depth("1 Introduction"), Some(1));
        assert_eq!(numbering_depth("1.2 Methods"), Some(2));
        assert_eq!(numbering_depth("1.2.3 Sub-method"), Some(3));
        assert_eq!(numbering_depth("A Appendix"), Some(1));
        assert_eq!(numbering_depth("A.1 First section"), Some(2));
        assert_eq!(numbering_depth("A.1.2 Detail"), Some(3));
        assert_eq!(numbering_depth("Bibliography"), None);
        assert_eq!(numbering_depth("Preface"), None);
    }

    #[test]
    fn test_hierarchy_from_numbering_only() {
        let entries = vec![
            make_entry("1 Introduction", 72.0, 12.0),
            make_entry("1.1 Background", 72.0, 12.0),
            make_entry("1.2 Motivation", 72.0, 12.0),
            make_entry("2 Methods", 72.0, 12.0),
        ];

        let config = HierarchyConfig::default();
        let result = assign_hierarchy(&entries, &config);

        assert_eq!(result[0].depth, 1); // "1 Introduction"
        assert_eq!(result[1].depth, 2); // "1.1 Background"
        assert_eq!(result[2].depth, 2); // "1.2 Motivation"
        assert_eq!(result[3].depth, 1); // "2 Methods"
    }

    #[test]
    fn test_hierarchy_from_indent() {
        // No numbering, rely on x-position.
        let entries = vec![
            make_entry("Introduction", 72.0, 14.0),
            make_entry("Background", 90.0, 12.0),
            make_entry("Details", 108.0, 10.0),
            make_entry("Methods", 72.0, 14.0),
        ];

        let config = HierarchyConfig {
            max_depth: 3,
            numbering_weight: 0.0, // disable numbering (all entries have None)
            indent_weight: 0.7,
            font_size_weight: 0.3,
        };
        let result = assign_hierarchy(&entries, &config);

        assert_eq!(result[0].depth, 1); // leftmost indent
        assert_eq!(result[1].depth, 2); // middle indent
        assert_eq!(result[2].depth, 3); // deepest indent
        assert_eq!(result[3].depth, 1); // back to leftmost
    }

    #[test]
    fn test_mixed_signals_numbering_wins() {
        // Numbering says depth 2, indent says depth 1 — numbering should win.
        let entries = vec![
            make_entry("1 Chapter", 72.0, 14.0),
            make_entry("1.1 Section", 72.0, 14.0), // same indent/font but numbered 1.1
        ];

        let config = HierarchyConfig::default();
        let result = assign_hierarchy(&entries, &config);

        assert_eq!(result[0].depth, 1);
        assert_eq!(result[1].depth, 2); // numbering wins
    }

    #[test]
    fn test_sanity_no_depth_skip() {
        // Manually construct entries that would produce a depth jump.
        let entries = vec![
            make_entry("1 Chapter", 72.0, 14.0),
            make_entry("1.1.1 Sub-sub", 72.0, 14.0), // depth 3 directly after depth 1
        ];

        let config = HierarchyConfig::default();
        let result = assign_hierarchy(&entries, &config);

        // Depth 3 should be clamped to 2 (can't jump from 1 to 3).
        assert_eq!(result[0].depth, 1);
        assert_eq!(result[1].depth, 2);
    }
}
