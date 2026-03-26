//! Base PDF file operations — atomic, composable, project-agnostic.
//!
//! These are the building blocks used by higher-level workflow commands.
//! Each function wraps a single `crate::pdf` module function with CLI I/O.
//!
//! # Base operations
//!
//! | Command              | Underlying function                        |
//! |----------------------|--------------------------------------------|
//! | `merge`              | `pdf::ops::merge`                          |
//! | `split`              | `pdf::ops::split`                          |
//! | `rotate`             | `pdf::ops::rotate`                         |
//! | `protect`            | `pdf::ops::encrypt`                        |
//! | `unlock`             | `pdf::ops::decrypt`                        |
//! | `inject-outlines`    | `pdf::heuristics::inject_outlines`         |
//! | `extract-pages`      | `bridge::pdf::extract_pages`               |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::helpers::parse_page_ranges;

// ---------------------------------------------------------------------------
// Structural operations (lossless, no re-encoding)
// ---------------------------------------------------------------------------

pub fn cmd_merge(output: PathBuf, inputs: Vec<PathBuf>) -> Result<()> {
    let input_refs: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();
    crate::pdf::ops::merge(&input_refs, &output)?;
    println!(
        "[arcane] Merged {} files → {}",
        inputs.len(),
        output.display()
    );
    Ok(())
}

pub fn cmd_split(input: PathBuf, output_dir: PathBuf, range_strs: Vec<String>) -> Result<()> {
    let ranges = parse_page_ranges(&range_strs)?;
    let paths = crate::pdf::ops::split(&input, &ranges, &output_dir)?;
    println!("[arcane] Split into {} file(s):", paths.len());
    for p in &paths {
        println!("  {}", p.display());
    }
    Ok(())
}

pub fn cmd_rotate(
    input: PathBuf,
    degrees: i32,
    output: Option<PathBuf>,
    pages: Vec<u32>,
) -> Result<()> {
    let out = output.unwrap_or_else(|| input.clone());
    crate::pdf::ops::rotate(&input, &pages, degrees, &out)?;
    println!("[arcane] Rotated {} → {}", input.display(), out.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Encryption
// ---------------------------------------------------------------------------

pub fn cmd_protect(input: PathBuf, password: &str, output: Option<PathBuf>) -> Result<()> {
    let out = output.unwrap_or_else(|| input.clone());
    crate::pdf::ops::encrypt(&input, password, &out)?;
    println!("[arcane] Encrypted {} → {}", input.display(), out.display());
    Ok(())
}

pub fn cmd_unlock(input: PathBuf, password: &str, output: Option<PathBuf>) -> Result<()> {
    let out = output.unwrap_or_else(|| input.clone());
    crate::pdf::ops::decrypt(&input, password, &out)?;
    println!("[arcane] Decrypted {} → {}", input.display(), out.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Outline injection  (base operation — exposed directly by `arcane pdf inject-outlines`)
// ---------------------------------------------------------------------------

/// Inject outline bookmarks from a JSON chapter-map file into a PDF.
///
/// The JSON file must contain an object mapping physical 0-based page indices
/// (as string keys) to chapter titles, e.g. `{"18": "Chapter 1", "44": "Chapter 2"}`.
///
/// This is the atomic operation that `arcane chunk` calls internally after
/// detecting chapter boundaries.
pub fn cmd_inject_outlines(
    input: PathBuf,
    chapters_json: PathBuf,
    output: Option<PathBuf>,
) -> Result<()> {
    let json_bytes = std::fs::read(&chapters_json)
        .with_context(|| format!("failed to read chapters JSON: {}", chapters_json.display()))?;

    // Accept both string-keyed and u32-keyed JSON maps.
    let raw: serde_json::Value = serde_json::from_slice(&json_bytes)
        .context("failed to parse chapters JSON — expected {\"page\": \"title\", ...}")?;

    let map = raw
        .as_object()
        .context("chapters JSON root must be an object")?;

    let mut chapter_map: BTreeMap<u32, String> = BTreeMap::new();
    for (k, v) in map {
        let page: u32 = k
            .parse()
            .with_context(|| format!("invalid page key in chapters JSON: {k:?}"))?;
        let title = v
            .as_str()
            .with_context(|| format!("title value for page {k} must be a string"))?
            .to_string();
        chapter_map.insert(page, title);
    }

    let mut doc = lopdf::Document::load(&input)
        .with_context(|| format!("failed to open PDF: {}", input.display()))?;

    let count = crate::pdf::heuristics::inject_outlines(&mut doc, &chapter_map)
        .context("outline injection failed")?;

    let out = output.unwrap_or_else(|| input.clone());
    doc.save(&out)
        .with_context(|| format!("failed to save PDF to {}", out.display()))?;

    println!(
        "[arcane] Injected {count} outline entries into {}",
        out.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Page extraction  (base operation — exposed directly by `arcane pdf extract-pages`)
// ---------------------------------------------------------------------------

/// Extract a page range from a PDF into a new file.
///
/// Page numbers are 1-based physical indices (as shown in your PDF reader).
/// This is the same operation used internally by `arcane process-toc` and
/// `arcane recover` to pull out TOC pages before OCR.
pub fn cmd_extract_pages(
    input: PathBuf,
    start: u32,
    end: u32,
    output: PathBuf,
) -> Result<()> {
    crate::bridge::pdf::extract_pages(&input, start, end, &output)
        .with_context(|| {
            format!(
                "failed to extract pages {start}-{end} from {}",
                input.display()
            )
        })?;
    println!(
        "[arcane] Extracted pages {start}-{end} from {} → {}",
        input.display(),
        output.display()
    );
    Ok(())
}
