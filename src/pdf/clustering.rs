//! 1D font-size clustering using Jenks natural breaks.
//!
//! Replaces the linear threshold chain in `heuristics::extract_headings` with
//! a data-driven approach that groups font sizes into clusters and assigns
//! semantic roles (Body, Heading1, Heading2, …) based on character counts and
//! relative size.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A raw cluster produced by the natural-breaks algorithm.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Cluster {
    /// Representative (centroid) font size for this cluster.
    pub centroid: f32,
    /// Members: `(font_size, char_count)`.
    pub members: Vec<(f32, u64)>,
    /// Total characters in this cluster.
    pub total_chars: u64,
}

/// A cluster with a semantic role assigned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontCluster {
    /// Representative font size (centroid of the cluster).
    pub centroid: f32,
    /// Total characters rendered at sizes within this cluster.
    pub char_count: u64,
    /// Inferred semantic role.
    pub role: FontRole,
}

/// Semantic role of a font-size cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    Body,
    Heading1,
    Heading2,
    Heading3,
    Footnote,
    PageNumber,
    Unknown,
}

// ---------------------------------------------------------------------------
// Clustering algorithm
// ---------------------------------------------------------------------------

/// Perform 1D natural-breaks (Jenks) clustering on a font-size histogram.
///
/// * `histogram` — from `build_font_histogram`: key = `font_size × 10` rounded,
///    value = total character count at that size.
/// * `max_k` — maximum number of clusters to produce.
///
/// Returns clusters sorted by centroid **descending** (largest font first).
/// Always returns at least 1 cluster for non-empty input.
pub fn cluster_font_sizes(histogram: &BTreeMap<u16, u64>, max_k: usize) -> Vec<Cluster> {
    if histogram.is_empty() {
        return vec![];
    }

    // Convert histogram to a sorted list of (font_size_f32, char_count).
    let mut values: Vec<(f32, u64)> = histogram
        .iter()
        .map(|(&key, &count)| (key as f32 / 10.0, count))
        .collect();
    values.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let n = values.len();
    if n == 1 || max_k <= 1 {
        return vec![make_cluster(&values)];
    }

    let k = max_k.min(n);

    // Find the optimal number of clusters (2..=k) using goodness-of-variance fit.
    let best_k = find_best_k(&values, k);
    let breaks = jenks_breaks(&values, best_k);

    // Split values into clusters at the break points.
    let mut clusters = Vec::with_capacity(best_k);
    let mut start = 0;
    for &brk in &breaks {
        if start < n && brk <= n {
            let slice = &values[start..brk];
            if !slice.is_empty() {
                clusters.push(make_cluster(slice));
            }
            start = brk;
        }
    }
    if start < n {
        clusters.push(make_cluster(&values[start..]));
    }

    // Sort descending by centroid (largest font first).
    clusters.sort_by(|a, b| b.centroid.partial_cmp(&a.centroid).unwrap_or(std::cmp::Ordering::Equal));

    clusters
}

/// Assign semantic roles to clusters.
///
/// * The cluster with the highest total character count is **Body**.
/// * Clusters with centroids **larger** than Body become Heading1, Heading2, …
///   (ordered by descending centroid).
/// * Clusters with centroids **smaller** than Body become Footnote (smallest)
///   or Unknown.
pub fn assign_roles(clusters: &[Cluster]) -> Vec<FontCluster> {
    if clusters.is_empty() {
        return vec![];
    }

    // Find the body cluster (highest char count).
    let body_idx = clusters
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| c.total_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let body_centroid = clusters[body_idx].centroid;

    // Separate into above-body and below-body groups.
    let mut above: Vec<(usize, f32)> = clusters
        .iter()
        .enumerate()
        .filter(|(i, c)| *i != body_idx && c.centroid > body_centroid)
        .map(|(i, c)| (i, c.centroid))
        .collect();
    above.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut below: Vec<(usize, f32)> = clusters
        .iter()
        .enumerate()
        .filter(|(i, c)| *i != body_idx && c.centroid < body_centroid)
        .map(|(i, c)| (i, c.centroid))
        .collect();
    below.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut result: Vec<FontCluster> = clusters
        .iter()
        .map(|c| FontCluster {
            centroid: c.centroid,
            char_count: c.total_chars,
            role: FontRole::Unknown,
        })
        .collect();

    // Assign body.
    result[body_idx].role = FontRole::Body;

    // Assign heading levels to above-body clusters (largest → Heading1, etc.).
    let heading_roles = [FontRole::Heading1, FontRole::Heading2, FontRole::Heading3];
    for (rank, &(idx, _)) in above.iter().enumerate() {
        result[idx].role = heading_roles.get(rank).copied().unwrap_or(FontRole::Unknown);
    }

    // Assign below-body roles.
    for (rank, &(idx, _)) in below.iter().enumerate() {
        result[idx].role = if rank == 0 {
            FontRole::Footnote
        } else {
            FontRole::PageNumber
        };
    }

    result
}

// ---------------------------------------------------------------------------
// Jenks internals
// ---------------------------------------------------------------------------

/// Find the best k (number of clusters) from 2..=max_k using GVF.
fn find_best_k(values: &[(f32, u64)], max_k: usize) -> usize {
    if max_k <= 2 {
        return max_k.max(1);
    }

    let sdam = sum_of_squared_deviations_from_mean(values);
    if sdam <= f64::EPSILON {
        return 1;
    }

    let mut best_k = 2;
    let mut best_gvf: f64 = 0.0;

    for k in 2..=max_k {
        let breaks = jenks_breaks(values, k);
        let sdcm = sum_of_squared_deviations_from_class_means(values, &breaks);
        let gvf = (sdam - sdcm) / sdam;

        // Accept this k if GVF improves by at least 0.02 (diminishing returns).
        if gvf > best_gvf + 0.02 {
            best_gvf = gvf;
            best_k = k;
        }
    }

    best_k
}

/// Compute optimal break indices for `k` clusters using the Jenks algorithm.
///
/// Returns a vector of `k-1` break indices into `values` (sorted ascending).
/// Each break index is the start of the next cluster.
fn jenks_breaks(values: &[(f32, u64)], k: usize) -> Vec<usize> {
    let n = values.len();
    if k >= n {
        // Each value is its own cluster.
        return (1..n).collect();
    }
    if k <= 1 {
        return vec![];
    }

    // Use weighted values (font_size weighted by char_count) for variance.
    let weighted: Vec<f64> = values.iter().map(|(s, _)| *s as f64).collect();

    // DP: cost[i][j] = minimum sum of squared deviations for clustering
    // values[0..=j] into i classes.
    let mut cost = vec![vec![f64::MAX; n]; k + 1];
    let mut split = vec![vec![0usize; n]; k + 1];

    // Base case: 1 cluster.
    for j in 0..n {
        cost[1][j] = ssd_range(&weighted, 0, j + 1);
    }

    // Fill DP for 2..=k clusters.
    for i in 2..=k {
        for j in (i - 1)..n {
            for m in (i - 2)..j {
                let c = cost[i - 1][m] + ssd_range(&weighted, m + 1, j + 1);
                if c < cost[i][j] {
                    cost[i][j] = c;
                    split[i][j] = m + 1;
                }
            }
        }
    }

    // Trace back to find break points.
    let mut breaks = Vec::with_capacity(k - 1);
    let mut j = n - 1;
    for i in (2..=k).rev() {
        breaks.push(split[i][j]);
        if split[i][j] > 0 {
            j = split[i][j] - 1;
        }
    }
    breaks.reverse();
    breaks
}

/// Sum of squared deviations from the overall mean.
fn sum_of_squared_deviations_from_mean(values: &[(f32, u64)]) -> f64 {
    let vals: Vec<f64> = values.iter().map(|(s, _)| *s as f64).collect();
    ssd_range(&vals, 0, vals.len())
}

/// Sum of squared deviations from class means, given break indices.
fn sum_of_squared_deviations_from_class_means(
    values: &[(f32, u64)],
    breaks: &[usize],
) -> f64 {
    let vals: Vec<f64> = values.iter().map(|(s, _)| *s as f64).collect();
    let n = vals.len();
    let mut total = 0.0;
    let mut start = 0;
    for &brk in breaks {
        if start < brk && brk <= n {
            total += ssd_range(&vals, start, brk);
            start = brk;
        }
    }
    if start < n {
        total += ssd_range(&vals, start, n);
    }
    total
}

/// Sum of squared deviations from mean for a slice `values[start..end]`.
fn ssd_range(values: &[f64], start: usize, end: usize) -> f64 {
    let slice = &values[start..end];
    if slice.is_empty() {
        return 0.0;
    }
    let mean = slice.iter().sum::<f64>() / slice.len() as f64;
    slice.iter().map(|v| (v - mean).powi(2)).sum()
}

/// Build a `Cluster` from a slice of `(font_size, char_count)`.
fn make_cluster(members: &[(f32, u64)]) -> Cluster {
    let total_chars: u64 = members.iter().map(|(_, c)| c).sum();
    let weighted_sum: f64 = members
        .iter()
        .map(|(s, c)| *s as f64 * *c as f64)
        .sum();
    let centroid = if total_chars > 0 {
        (weighted_sum / total_chars as f64) as f32
    } else {
        members.iter().map(|(s, _)| *s).sum::<f32>() / members.len() as f32
    };

    Cluster {
        centroid,
        members: members.to_vec(),
        total_chars,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_single_size() {
        let mut h = BTreeMap::new();
        h.insert(120u16, 10_000u64);
        let clusters = cluster_font_sizes(&h, 5);
        assert_eq!(clusters.len(), 1);
        assert!((clusters[0].centroid - 12.0).abs() < 0.1);
    }

    #[test]
    fn cluster_two_groups() {
        let mut h = BTreeMap::new();
        // Body text: 12pt (lots of chars)
        h.insert(120, 50_000);
        // Headings: 18pt (few chars)
        h.insert(180, 500);
        let clusters = cluster_font_sizes(&h, 5);
        assert!(clusters.len() >= 1);
        // With only 2 distinct sizes, we should get 2 clusters.
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn cluster_three_distinct_sizes() {
        let mut h = BTreeMap::new();
        h.insert(100, 5_000);   // 10pt — footnote
        h.insert(120, 50_000);  // 12pt — body
        h.insert(180, 200);     // 18pt — heading
        h.insert(200, 150);     // 20pt — heading (close to 18pt)

        let clusters = cluster_font_sizes(&h, 5);
        assert!(clusters.len() >= 2);
    }

    #[test]
    fn assign_roles_body_is_most_frequent() {
        let clusters = vec![
            Cluster {
                centroid: 18.0,
                members: vec![(18.0, 200)],
                total_chars: 200,
            },
            Cluster {
                centroid: 12.0,
                members: vec![(12.0, 50_000)],
                total_chars: 50_000,
            },
            Cluster {
                centroid: 8.0,
                members: vec![(8.0, 1_000)],
                total_chars: 1_000,
            },
        ];

        let roles = assign_roles(&clusters);
        assert_eq!(roles.len(), 3);

        // Body should be 12pt (most chars).
        let body = roles.iter().find(|r| r.role == FontRole::Body).unwrap();
        assert!((body.centroid - 12.0).abs() < 0.1);

        // Heading1 should be 18pt (above body).
        let h1 = roles.iter().find(|r| r.role == FontRole::Heading1).unwrap();
        assert!((h1.centroid - 18.0).abs() < 0.1);

        // Footnote should be 8pt (below body).
        let fn_ = roles.iter().find(|r| r.role == FontRole::Footnote).unwrap();
        assert!((fn_.centroid - 8.0).abs() < 0.1);
    }

    #[test]
    fn empty_histogram_returns_empty() {
        let h = BTreeMap::new();
        assert!(cluster_font_sizes(&h, 5).is_empty());
    }

    #[test]
    fn assign_roles_empty() {
        assert!(assign_roles(&[]).is_empty());
    }
}
