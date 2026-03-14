//! PDF intelligence and chunking engine.
//!
//! # Logical vs. Physical page alignment
//!
//! A PDF file numbers its pages *physically* starting at index 0.  Textbooks
//! often have front-matter (preface, table of contents) that uses Roman
//! numerals, so the printed "Page 1" may only appear at physical index 12.
//!
//! We call the difference the **offset**:
//! ```text
//! offset = physical_index - logical_page_number
//! ```
//!
//! Given an offset we can convert a human-supplied logical page range
//! `[logical_start, logical_end)` to the correct physical slice.
//!
//! # Chapter boundary detection strategy ("Xodo Method")
//!
//! 1. **`/Outlines` (Table of Contents tree)** — preferred when present.
//!    Each top-level `Dest` or `GoTo` action in the outline tree carries the
//!    physical page object reference, letting us derive exact boundary pages.
//!
//! 2. **`/PageLabels` dictionary** — fallback.  The PDF standard allows a
//!    `PageLabels` number tree that maps ranges of physical pages to label
//!    styles.  We use the *start* of each labelled run as a chapter boundary.
//!
//! 3. **User-supplied `start_page_physical`** — last resort.  When neither
//!    Outlines nor PageLabels are available the user provides the physical
//!    index where the printed "Page 1" starts.  The engine then uses whatever
//!    `chapter_map` the user supplied (mapping physical → title) or, when that
//!    too is empty, produces a single chunk for the whole document.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use lopdf::{Document, Object, ObjectId};

use crate::models::SourceMeta;

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Split the PDF described by `meta` and write per-chapter files into
/// `chunks_dir`.
///
/// The function is **idempotent**: if `chunks_dir` already contains `.pdf`
/// files it returns early without re-processing.
pub fn chunk_pdf(meta: &SourceMeta, chunks_dir: &Path) -> anyhow::Result<()> {
    // ── Idempotency guard ────────────────────────────────────────────────────
    if is_already_chunked(chunks_dir)? {
        println!(
            "[arcane] Chunks directory '{}' is already populated — skipping.",
            chunks_dir.display()
        );
        return Ok(());
    }

    println!(
        "[arcane] Loading PDF '{}'…",
        meta.path.display()
    );

    let doc = Document::load(&meta.path)
        .with_context(|| format!("failed to open PDF at {}", meta.path.display()))?;

    let total_pages = doc.get_pages().len() as u32;
    println!("[arcane] Total physical pages: {total_pages}");

    // ── Determine chapter boundaries ─────────────────────────────────────────
    // BTreeMap<physical_start_index (0-based), chapter_title>
    let chapters: BTreeMap<u32, String> = if !meta.chapter_map.is_empty() {
        // The user provided an explicit mapping — use it directly.
        meta.chapter_map.iter().map(|(&k, v)| (k, v.clone())).collect()
    } else {
        // Try automatic detection.
        extract_chapters_from_outlines(&doc)
            .or_else(|_| extract_chapters_from_page_labels(&doc))
            .unwrap_or_default()
    };

    // ── Fallback: treat whole document as one chunk ───────────────────────────
    let chapters = if chapters.is_empty() {
        let title = meta.title.clone();
        let mut m = BTreeMap::new();
        m.insert(0u32, title);
        m
    } else {
        chapters
    };

    // ── Build page ranges from boundary map ──────────────────────────────────
    let ranges = boundaries_to_ranges(&chapters, total_pages);

    // ── Write individual chapter PDFs ─────────────────────────────────────────
    fs::create_dir_all(chunks_dir)
        .with_context(|| format!("cannot create chunks dir {}", chunks_dir.display()))?;

    for (idx, (start, end, title)) in ranges.iter().enumerate() {
        let safe_title = sanitise_filename(title);
        let filename = format!("{:02}_{}.pdf", idx + 1, safe_title);
        let out_path = chunks_dir.join(&filename);

        println!("[arcane] Writing chunk {filename} (pages {start}–{end})…");
        write_chunk(&doc, *start, *end, &out_path)?;
    }

    println!("[arcane] Done — {} chunk(s) written.", ranges.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

fn is_already_chunked(dir: &Path) -> Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    let has_pdf = fs::read_dir(dir)
        .with_context(|| format!("cannot list directory {}", dir.display()))?
        .filter_map(|e| e.ok())
        .any(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false)
        });
    Ok(has_pdf)
}

// ---------------------------------------------------------------------------
// Chapter detection — /Outlines
// ---------------------------------------------------------------------------

/// Walk the PDF Outlines (bookmarks) tree and collect the first-level entries.
///
/// Each entry contributes one boundary: its resolved physical page index and
/// the bookmark title.  Pages are returned as 0-based physical indices.
fn extract_chapters_from_outlines(doc: &Document) -> Result<BTreeMap<u32, String>> {
    let catalog = doc.catalog()?;

    let outlines_ref = match catalog.get(b"Outlines") {
        Ok(obj) => obj.as_reference()?,
        Err(_) => bail!("no /Outlines in catalog"),
    };

    let outlines = doc.get_object(outlines_ref)?;
    let outlines_dict = outlines.as_dict()?;

    let first_ref = match outlines_dict.get(b"First") {
        Ok(obj) => obj.as_reference()?,
        Err(_) => bail!("empty /Outlines"),
    };

    let mut result = BTreeMap::new();
    let mut current_ref: Option<ObjectId> = Some(first_ref);

    while let Some(item_ref) = current_ref {
        let item = doc.get_object(item_ref)?;
        let item_dict = item.as_dict()?;

        // Extract the title string.
        if let Ok(title_obj) = item_dict.get(b"Title") {
            let title = pdf_string_to_string(title_obj);

            // Resolve the destination page.
            if let Some(page_idx) = resolve_dest_page(doc, item_dict) {
                result.insert(page_idx, title);
            }
        }

        // Advance to the next sibling.
        current_ref = item_dict
            .get(b"Next")
            .ok()
            .and_then(|o| o.as_reference().ok());
    }

    if result.is_empty() {
        bail!("no usable entries in /Outlines");
    }
    Ok(result)
}

/// Attempt to resolve the physical page index (0-based) from an outline item.
fn resolve_dest_page(doc: &Document, item_dict: &lopdf::Dictionary) -> Option<u32> {
    // Try /Dest array first.
    if let Ok(dest) = item_dict.get(b"Dest") {
        return page_from_dest(doc, dest);
    }

    // Try /A (action dict) with /S /GoTo.
    if let Ok(action) = item_dict.get(b"A") {
        if let Ok(a_dict) = action.as_dict() {
            if let Ok(dest) = a_dict.get(b"D") {
                return page_from_dest(doc, dest);
            }
        }
    }
    None
}

fn page_from_dest(doc: &Document, dest: &Object) -> Option<u32> {
    match dest {
        Object::Array(array) => {
            // First element of the destination array is the page object reference.
            let page_ref = array.first()?.as_reference().ok()?;
            let pages = doc.get_pages();
            // get_pages returns a BTreeMap<page_number (1-based), ObjectId>.
            let page_number = pages
                .iter()
                .find(|(_, &oid)| oid == page_ref)
                .map(|(&num, _)| num)?;
            Some(page_number.saturating_sub(1)) // convert to 0-based
        }
        Object::Reference(r) => {
            // Named/indirect destination — dereference and recurse once.
            let inner = doc.get_object(*r).ok()?;
            page_from_dest(doc, inner)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Chapter detection — /PageLabels
// ---------------------------------------------------------------------------

/// Parse `/PageLabels` and treat each range start as a chapter boundary.
fn extract_chapters_from_page_labels(doc: &Document) -> Result<BTreeMap<u32, String>> {
    let catalog = doc.catalog()?;

    let labels_obj = catalog
        .get(b"PageLabels")
        .context("no /PageLabels in catalog")?;

    // /PageLabels is a number tree.  The most common form (after lopdf loads it)
    // is a dictionary with /Nums key containing an array [n0, label0, n1, label1 …].
    let nums_array: Vec<Object> = match labels_obj {
        Object::Dictionary(d) => d
            .get(b"Nums")
            .context("no Nums in PageLabels")?
            .as_array()
            .context("Nums is not an array")?
            .clone(),
        Object::Array(a) => a.clone(),
        Object::Reference(r) => {
            let inner = doc.get_object(*r)?;
            match inner {
                Object::Dictionary(d) => d
                    .get(b"Nums")
                    .context("no Nums")?
                    .as_array()
                    .context("Nums not array")?
                    .clone(),
                _ => bail!("unexpected PageLabels structure"),
            }
        }
        _ => bail!("unexpected PageLabels type"),
    };

    let mut result = BTreeMap::new();
    let mut i = 0;
    while i + 1 < nums_array.len() {
        let start_page = match &nums_array[i] {
            Object::Integer(n) => *n as u32,
            _ => {
                i += 2;
                continue;
            }
        };
        let label = match &nums_array[i + 1] {
            Object::Dictionary(d) => {
                // The label dict may contain a /P (prefix) entry.
                d.get(b"P")
                    .ok()
                    .and_then(|o| pdf_string_to_string_opt(o))
                    .unwrap_or_else(|| format!("Section starting at page {start_page}"))
            }
            _ => format!("Section starting at page {start_page}"),
        };
        result.insert(start_page, label);
        i += 2;
    }

    if result.is_empty() {
        bail!("no usable entries in /PageLabels");
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Range builder
// ---------------------------------------------------------------------------

/// Convert a map of `start_page → title` into a list of `(start, end, title)`
/// triples, where `end` is the last physical page index (inclusive) in the
/// chapter.
fn boundaries_to_ranges(
    chapters: &BTreeMap<u32, String>,
    total_pages: u32,
) -> Vec<(u32, u32, String)> {
    let keys: Vec<u32> = chapters.keys().copied().collect();
    let mut ranges = Vec::new();
    for (i, &start) in keys.iter().enumerate() {
        let end = if i + 1 < keys.len() {
            keys[i + 1].saturating_sub(1)
        } else {
            total_pages.saturating_sub(1)
        };
        let title = chapters[&start].clone();
        ranges.push((start, end, title));
    }
    ranges
}

// ---------------------------------------------------------------------------
// PDF writer
// ---------------------------------------------------------------------------

/// Extract pages `[start, end]` (inclusive, 0-based) from `doc` and write a
/// new minimal PDF to `out_path`.
///
/// This uses `lopdf`'s page extraction facilities without re-encoding streams,
/// preserving quality and keeping the operation fast.
fn write_chunk(doc: &Document, start: u32, end: u32, out_path: &Path) -> Result<()> {
    // lopdf uses 1-based page numbers in its public API.
    let first = start + 1;
    let last = end + 1;

    // Build a list of 1-based page numbers to keep.
    let pages_to_keep: Vec<u32> = (first..=last).collect();

    if pages_to_keep.is_empty() {
        bail!("empty page range ({start}–{end})");
    }

    // Clone the document so we can delete pages without mutating the original.
    let mut chunk_doc = doc.clone();

    let total = chunk_doc.get_pages().len() as u32;

    // Pages to delete = all pages NOT in our range.
    let pages_to_delete: Vec<u32> = (1..=total)
        .filter(|p| !pages_to_keep.contains(p))
        .collect();

    // Batch-delete all unwanted pages in one call.
    chunk_doc.delete_pages(&pages_to_delete);

    chunk_doc
        .save(out_path)
        .with_context(|| format!("failed to save chunk to {}", out_path.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Decode a PDF string/name object into a Rust `String`.
fn pdf_string_to_string(obj: &Object) -> String {
    match obj {
        Object::String(bytes, _) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        Object::Name(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

fn pdf_string_to_string_opt(obj: &Object) -> Option<String> {
    let s = pdf_string_to_string(obj);
    if s.is_empty() { None } else { Some(s) }
}

/// Replace characters that are problematic in file names.
fn sanitise_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
        .replace("  ", " ")
        .replace(' ', "_")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn boundaries_single_chapter() {
        let mut m = BTreeMap::new();
        m.insert(0u32, "Intro".into());
        let ranges = boundaries_to_ranges(&m, 20);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (0, 19, "Intro".to_string()));
    }

    #[test]
    fn boundaries_multiple_chapters() {
        let mut m = BTreeMap::new();
        m.insert(0u32, "Front Matter".into());
        m.insert(10u32, "Chapter 1".into());
        m.insert(25u32, "Chapter 2".into());
        let ranges = boundaries_to_ranges(&m, 40);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], (0, 9, "Front Matter".to_string()));
        assert_eq!(ranges[1], (10, 24, "Chapter 1".to_string()));
        assert_eq!(ranges[2], (25, 39, "Chapter 2".to_string()));
    }

    #[test]
    fn sanitise_special_chars() {
        assert_eq!(sanitise_filename("Hello / World"), "Hello___World");
        assert_eq!(sanitise_filename("Chapter 1: Intro"), "Chapter_1__Intro");
        assert_eq!(sanitise_filename("Normal Title"), "Normal_Title");
    }

    #[test]
    fn already_chunked_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_already_chunked(dir.path()).unwrap());
    }

    #[test]
    fn already_chunked_with_pdf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("01_chapter.pdf"), b"%PDF-1.4").unwrap();
        assert!(is_already_chunked(dir.path()).unwrap());
    }

    #[test]
    fn already_chunked_no_pdf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"hello").unwrap();
        assert!(!is_already_chunked(dir.path()).unwrap());
    }
}
