//! Shared parsing utilities used across CLI command modules.

use std::path::PathBuf;

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Public parsers
// ---------------------------------------------------------------------------

/// Parse a `"LOGICAL:PHYSICAL"` anchor string into a `(logical_1based, physical_1based)` pair.
///
/// Referenced by `src/cli/mod.rs` as `crate::cli::commands::parse_anchor_pair` in the
/// `#[arg(value_parser = ...)]` attribute — must stay `pub` at this exact path.
pub fn parse_anchor_pair(s: &str) -> Result<(u32, u32), String> {
    let (l, r) = s
        .split_once(':')
        .ok_or_else(|| format!("expected LOGICAL:PHYSICAL, got {s:?}"))?;
    let logical: u32 = l.trim().parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
    let physical: u32 = r.trim().parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
    if logical == 0 || physical == 0 {
        return Err("page numbers must be >= 1".into());
    }
    Ok((logical, physical))
}

/// Parse a `"start-end"` range string (1-based) into a 0-based inclusive tuple.
///
/// Also accepts a single page number (`"N"` → `(N-1, N-1)`).
pub fn parse_page_range(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let start: u32 = parts[0].trim().parse().ok()?;
        let end: u32 = parts[1].trim().parse().ok()?;
        if start >= 1 && end >= start {
            return Some((start - 1, end - 1)); // convert to 0-based
        }
    } else if parts.len() == 1 {
        let page: u32 = parts[0].trim().parse().ok()?;
        if page >= 1 {
            return Some((page - 1, page - 1));
        }
    }
    None
}

/// Parse a slice of `"N-M"` / `"N"` range strings (1-based) into 0-based `(start, end)` tuples.
pub fn parse_page_ranges(range_strs: &[String]) -> Result<Vec<(u32, u32)>> {
    let mut ranges = Vec::new();
    for s in range_strs {
        let parts: Vec<&str> = s.split('-').collect();
        match parts.as_slice() {
            [start, end] => {
                let sv: u32 = start
                    .parse()
                    .with_context(|| format!("invalid page number in range '{s}'"))?;
                let ev: u32 = end
                    .parse()
                    .with_context(|| format!("invalid page number in range '{s}'"))?;
                if sv == 0 || ev == 0 {
                    anyhow::bail!("page numbers must be >= 1 in range '{s}'");
                }
                if sv > ev {
                    anyhow::bail!("invalid range '{s}': start > end");
                }
                ranges.push((sv - 1, ev - 1));
            }
            [single] => {
                let p: u32 = single
                    .parse()
                    .with_context(|| format!("invalid page number '{s}'"))?;
                if p == 0 {
                    anyhow::bail!("page numbers must be >= 1");
                }
                ranges.push((p - 1, p - 1));
            }
            _ => anyhow::bail!("invalid range format: '{s}'. Expected 'N' or 'N-M'."),
        }
    }
    Ok(ranges)
}

/// Parse a `"START-END"` TOC range string (1-based, inclusive, both ≥ 1).
pub fn parse_toc_range_1based(s: &str) -> Result<(u32, u32)> {
    let parts: Vec<&str> = s.splitn(2, '-').collect();
    if parts.len() != 2 {
        anyhow::bail!(
            "--toc-pages must be in START-END format (for example, 7-18), got: {s:?}"
        );
    }
    let start: u32 = parts[0]
        .trim()
        .parse()
        .with_context(|| format!("invalid start page in --toc-pages {s:?}"))?;
    let end: u32 = parts[1]
        .trim()
        .parse()
        .with_context(|| format!("invalid end page in --toc-pages {s:?}"))?;
    if start == 0 || start > end {
        anyhow::bail!("--toc-pages start must be >= 1 and <= end, got: {s:?}");
    }
    Ok((start, end))
}

/// Build a unique temporary file path under the OS temp directory.
pub fn temp_file_path(prefix: &str, extension: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "arcane-{prefix}-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ));
    path
}

/// Resolve the Arcane data root directory (`~/Arcane` by default).
pub fn resolve_arcane_data(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    let home = crate::storage::filesystem::dirs_home()
        .context("could not determine home directory; pass --arcane-data explicitly")?;
    Ok(home.join("Arcane"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{parse_page_range, parse_toc_range_1based};

    #[test]
    fn parse_toc_range_accepts_valid_input() {
        let parsed = parse_toc_range_1based("7-18").expect("expected valid range");
        assert_eq!(parsed, (7, 18));
    }

    #[test]
    fn parse_toc_range_rejects_zero_start() {
        assert!(parse_toc_range_1based("0-5").is_err());
    }

    #[test]
    fn parse_toc_range_rejects_reversed_range() {
        assert!(parse_toc_range_1based("12-4").is_err());
    }

    #[test]
    fn parse_page_range_converts_to_zero_based() {
        assert_eq!(parse_page_range("3-5"), Some((2, 4)));
        assert_eq!(parse_page_range("9"), Some((8, 8)));
    }
}
