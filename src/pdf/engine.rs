//! Core PDF chunking engine.
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
//! # Chapter boundary detection strategy ("Xodo Method")
//!
//! 1. **`/Outlines` (Table of Contents tree)** — preferred when present.
//! 2. **`/PageLabels` dictionary** — fallback.
//! 3. **User-supplied `start_page_physical`** — last resort.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use lopdf::{Document, Object};
use rayon::prelude::*;

use super::outlines::extract_chapters_from_outlines;
use super::page_labels::extract_chapters_from_page_labels;
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
        tracing::info!(
            "Chunks directory '{}' is already populated — skipping.",
            chunks_dir.display()
        );
        return Ok(());
    }

    tracing::info!("Loading PDF '{}'…", meta.path.display());

    let doc = Document::load(&meta.path)
        .with_context(|| format!("failed to open PDF at {}", meta.path.display()))?;

    // Cache page map once — reused by outline detection and range building.
    let total_pages = doc.get_pages().len() as u32;
    tracing::info!("Total physical pages: {total_pages}");

    // ── Determine chapter boundaries ─────────────────────────────────────────
    // BTreeMap<physical_start_index (0-based), chapter_title>
    let chapters: BTreeMap<u32, String> = if !meta.chapter_map.is_empty() {
        // The user provided an explicit mapping — use it directly.
        meta.chapter_map
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect()
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

    // ── Write individual chapter PDFs (parallel) ──────────────────────────────
    fs::create_dir_all(chunks_dir)
        .with_context(|| format!("cannot create chunks dir {}", chunks_dir.display()))?;

    // Build the full page list once and share across threads via Arc.
    let doc = std::sync::Arc::new(doc);

    let write_jobs: Vec<(usize, u32, u32, String)> = ranges
        .into_iter()
        .enumerate()
        .map(|(idx, (s, e, t))| (idx, s, e, t))
        .collect();

    let errors: Vec<_> = write_jobs
        .par_iter()
        .filter_map(|(idx, start, end, title)| {
            let safe_title = sanitise_filename(title);
            let filename = format!("{:02}_{}.pdf", idx + 1, safe_title);
            let out_path = chunks_dir.join(&filename);
            tracing::info!("Writing chunk {filename} (pages {start}–{end})…");
            write_chunk(&doc, *start, *end, &out_path).err()
        })
        .collect();

    if let Some(first_err) = errors.into_iter().next() {
        return Err(first_err);
    }

    tracing::info!("Done — {} chunk(s) written.", write_jobs.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

pub(crate) fn is_already_chunked(dir: &Path) -> Result<bool> {
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
// Range builder
// ---------------------------------------------------------------------------

/// Convert a map of `start_page → title` into a list of `(start, end, title)`
/// triples, where `end` is the last physical page index (inclusive) in the
/// chapter.
pub(crate) fn boundaries_to_ranges(
    chapters: &BTreeMap<u32, String>,
    total_pages: u32,
) -> Vec<(u32, u32, String)> {
    let keys: Vec<u32> = chapters.keys().copied().collect();
    let last_valid = total_pages.saturating_sub(1);
    let mut ranges = Vec::new();
    for (i, &start) in keys.iter().enumerate() {
        // Skip entries whose start page is beyond the document.
        if start > last_valid {
            continue;
        }
        let end = if i + 1 < keys.len() {
            keys[i + 1].saturating_sub(1).min(last_valid)
        } else {
            last_valid
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
/// Avoids `doc.clone()` entirely. Instead, performs a BFS over the PDF object
/// graph starting from the target page objects, collecting only the objects
/// actually referenced by those pages (content streams, fonts, XObjects, …).
/// This means a 30-page chapter copies ~30 page objects + their resources
/// rather than all objects in the full document.
fn write_chunk(doc: &Document, start: u32, end: u32, out_path: &Path) -> Result<()> {
    use lopdf::{Dictionary, Object, ObjectId};
    use std::collections::{HashMap, HashSet, VecDeque};

    let first = start + 1; // lopdf page numbers are 1-based
    let last = end + 1;

    if first > last {
        bail!("empty page range ({start}–{end})");
    }

    let all_pages = doc.get_pages(); // BTreeMap<1-based page number, ObjectId>

    let target_page_ids: Vec<ObjectId> = (first..=last)
        .filter_map(|n| all_pages.get(&n).copied())
        .collect();

    if target_page_ids.is_empty() {
        bail!("no pages found for range {start}–{end}");
    }

    // ── BFS: collect all objects reachable from target pages ────────────────
    // We intentionally skip the "Parent" key on page dicts to avoid pulling
    // in the entire pages tree (which would include all sibling pages).
    let mut needed: HashSet<ObjectId> = HashSet::new();
    let mut queue: VecDeque<ObjectId> = target_page_ids.iter().copied().collect();

    while let Some(id) = queue.pop_front() {
        if !needed.insert(id) {
            continue;
        }
        if let Ok(obj) = doc.get_object(id) {
            // When the object is a page dictionary, skip "Parent" so we don't
            // traverse up to the Pages tree (and pull in every other page).
            let skip_parent = is_page_dict(obj);
            collect_refs(obj, skip_parent, &needed, &mut queue);
        }
    }

    // ── Copy needed objects into a new document with fresh IDs ───────────────
    let mut new_doc = Document::with_version("1.5");
    let mut id_map: HashMap<ObjectId, ObjectId> = HashMap::new();

    for &old_id in &needed {
        if let Ok(obj) = doc.get_object(old_id) {
            let new_id = new_doc.add_object(obj.clone());
            id_map.insert(old_id, new_id);
        }
    }

    // Remap all internal cross-references in the copied objects.
    let remapped_ids: Vec<ObjectId> = new_doc.objects.keys().copied().collect();
    for id in remapped_ids {
        if let Some(obj) = new_doc.objects.get_mut(&id) {
            remap_refs(obj, &id_map);
        }
    }

    // ── Build a fresh Pages tree for just our target pages ───────────────────
    let new_page_ids: Vec<ObjectId> = target_page_ids
        .iter()
        .filter_map(|old_id| id_map.get(old_id).copied())
        .collect();

    let kids: Vec<Object> = new_page_ids
        .iter()
        .map(|&id| Object::Reference(id))
        .collect();
    let count = new_page_ids.len() as i64;

    let pages_id = new_doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Pages".to_vec()));
        d.set("Kids", Object::Array(kids));
        d.set("Count", Object::Integer(count));
        d
    }));

    // Point every page's Parent to the new Pages node.
    for &page_id in &new_page_ids {
        if let Some(Object::Dictionary(dict)) = new_doc.objects.get_mut(&page_id) {
            dict.set("Parent", Object::Reference(pages_id));
        }
    }

    // ── Build catalog ────────────────────────────────────────────────────────
    let catalog_id = new_doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Catalog".to_vec()));
        d.set("Pages", Object::Reference(pages_id));
        d
    }));

    new_doc.trailer.set("Root", Object::Reference(catalog_id));

    new_doc
        .save(out_path)
        .with_context(|| format!("failed to save chunk to {}", out_path.display()))?;

    Ok(())
}

/// Return `true` if the object is a PDF page dictionary (`/Type /Page`).
fn is_page_dict(obj: &lopdf::Object) -> bool {
    if let Ok(dict) = obj.as_dict() {
        if let Ok(t) = dict.get(b"Type") {
            if let Ok(name) = t.as_name() {
                return name == b"Page";
            }
        }
    }
    false
}

/// Walk an object and push every ObjectId reference found into `queue`,
/// skipping those already in `visited`. When `skip_parent` is `true` the
/// `"Parent"` key of dictionaries is not followed (prevents traversing up the
/// pages tree from a page object).
fn collect_refs(
    obj: &lopdf::Object,
    skip_parent: bool,
    visited: &std::collections::HashSet<lopdf::ObjectId>,
    queue: &mut std::collections::VecDeque<lopdf::ObjectId>,
) {
    match obj {
        lopdf::Object::Reference(id) => {
            if !visited.contains(id) {
                queue.push_back(*id);
            }
        }
        lopdf::Object::Array(arr) => {
            for item in arr {
                collect_refs(item, false, visited, queue);
            }
        }
        lopdf::Object::Dictionary(dict) => {
            for (key, val) in dict.iter() {
                if skip_parent && key == b"Parent" {
                    continue;
                }
                collect_refs(val, false, visited, queue);
            }
        }
        lopdf::Object::Stream(stream) => {
            for (key, val) in stream.dict.iter() {
                if skip_parent && key == b"Parent" {
                    continue;
                }
                collect_refs(val, false, visited, queue);
            }
        }
        _ => {}
    }
}

/// Recursively remap every ObjectId reference inside `obj` using `id_map`.
/// References not present in `id_map` are left unchanged.
fn remap_refs(
    obj: &mut lopdf::Object,
    id_map: &std::collections::HashMap<lopdf::ObjectId, lopdf::ObjectId>,
) {
    match obj {
        lopdf::Object::Reference(id) => {
            if let Some(&new_id) = id_map.get(id) {
                *id = new_id;
            }
        }
        lopdf::Object::Array(arr) => {
            for item in arr.iter_mut() {
                remap_refs(item, id_map);
            }
        }
        lopdf::Object::Dictionary(dict) => {
            for (_, val) in dict.iter_mut() {
                remap_refs(val, id_map);
            }
        }
        lopdf::Object::Stream(stream) => {
            for (_, val) in stream.dict.iter_mut() {
                remap_refs(val, id_map);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Decode a PDF string/name object into a Rust `String`.
///
/// PDF strings may be:
/// - UTF-16-BE with a `\xFE\xFF` BOM (most modern PDF generators)
/// - PDFDocEncoding (Latin-1 superset) — treated as UTF-8 lossy
pub(crate) fn pdf_string_to_string(obj: &Object) -> String {
    match obj {
        Object::String(bytes, _) => decode_pdf_bytes(bytes),
        Object::Name(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

/// Decode raw PDF string bytes, handling UTF-16-BE BOM.
fn decode_pdf_bytes(bytes: &[u8]) -> String {
    // UTF-16-BE is indicated by the `\xFE\xFF` BOM at the start.
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // Collect u16 code units from big-endian byte pairs.
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        // PDFDocEncoding / UTF-8: treat as lossy UTF-8.
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub(crate) fn pdf_string_to_string_opt(obj: &Object) -> Option<String> {
    let s = pdf_string_to_string(obj);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Replace characters that are problematic in file names.
pub(crate) fn sanitise_filename(name: &str) -> String {
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
