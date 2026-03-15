//! Chapter detection from PDF `/Outlines` (bookmarks / table of contents).
//!
//! Walks the outline tree's top-level entries and collects each entry's
//! resolved physical page index and title as a chapter boundary.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use lopdf::{Document, Object, ObjectId};

use super::pdf_string_to_string; // UTF-16-BE aware

/// Walk the PDF Outlines (bookmarks) tree and collect entries up to `max_depth`
/// levels deep (1 = top-level only, which is the original behavior).
///
/// Returns 0-based physical page indices.
pub fn extract_chapters_from_outlines(doc: &Document) -> Result<BTreeMap<u32, String>> {
    extract_chapters_with_depth(doc, 1)
}

/// Walk the PDF Outlines tree collecting entries up to `max_depth` levels.
///
/// Depth 1 = top-level chapters only, 2 = chapters + sections, etc.
pub fn extract_chapters_with_depth(
    doc: &Document,
    max_depth: u32,
) -> Result<BTreeMap<u32, String>> {
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

    // Build the page map ONCE — ObjectId → 0-based physical page index.
    let oid_to_page: BTreeMap<ObjectId, u32> = doc
        .get_pages()
        .into_iter()
        .map(|(page_num, oid)| (oid, page_num.saturating_sub(1)))
        .collect();

    let mut result = BTreeMap::new();
    walk_outline_entries(
        doc,
        first_ref,
        &oid_to_page,
        1,
        max_depth,
        None,
        &mut result,
    );

    if result.is_empty() {
        bail!("no usable entries in /Outlines");
    }
    Ok(result)
}

/// Recursively walk outline entries at the current depth, following `Next`
/// for siblings and `First` for children (when depth allows).
fn walk_outline_entries(
    doc: &Document,
    first_ref: ObjectId,
    oid_to_page: &BTreeMap<ObjectId, u32>,
    current_depth: u32,
    max_depth: u32,
    parent_title: Option<&str>,
    result: &mut BTreeMap<u32, String>,
) {
    let mut current_ref: Option<ObjectId> = Some(first_ref);

    while let Some(item_ref) = current_ref {
        let item = match doc.get_object(item_ref) {
            Ok(obj) => obj,
            Err(_) => break,
        };
        let item_dict = match item.as_dict() {
            Ok(d) => d,
            Err(_) => break,
        };

        if let Ok(title_obj) = item_dict.get(b"Title") {
            let raw_title = pdf_string_to_string(title_obj);

            // Build qualified title for nested entries.
            let title = match parent_title {
                Some(parent) if current_depth > 1 => format!("{parent} > {raw_title}"),
                _ => raw_title.clone(),
            };

            if let Some(page_idx) = resolve_dest_page(doc, item_dict, oid_to_page) {
                result.insert(page_idx, title);
            }

            // Recurse into children if depth allows.
            if current_depth < max_depth {
                if let Ok(first_child) = item_dict.get(b"First") {
                    if let Ok(child_ref) = first_child.as_reference() {
                        walk_outline_entries(
                            doc,
                            child_ref,
                            oid_to_page,
                            current_depth + 1,
                            max_depth,
                            Some(&raw_title),
                            result,
                        );
                    }
                }
            }
        }

        current_ref = item_dict
            .get(b"Next")
            .ok()
            .and_then(|o| o.as_reference().ok());
    }
}

/// Attempt to resolve the physical page index (0-based) from an outline item.
fn resolve_dest_page(
    doc: &Document,
    item_dict: &lopdf::Dictionary,
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    if let Ok(dest) = item_dict.get(b"Dest") {
        return page_from_dest(doc, dest, oid_to_page);
    }
    if let Ok(action) = item_dict.get(b"A") {
        // /A may itself be an indirect reference — dereference before as_dict().
        let action_deref;
        let action_obj: &Object = if let Object::Reference(r) = action {
            action_deref = doc.get_object(*r).ok()?;
            action_deref
        } else {
            action
        };
        if let Ok(a_dict) = action_obj.as_dict() {
            if let Ok(dest) = a_dict.get(b"D") {
                return page_from_dest(doc, dest, oid_to_page);
            }
        }
    }
    None
}

fn page_from_dest(
    doc: &Document,
    dest: &Object,
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    match dest {
        // Direct destination: [page_ref /XYZ left top zoom] or similar
        Object::Array(array) => {
            let page_ref = array.first()?.as_reference().ok()?;
            oid_to_page.get(&page_ref).copied()
        }
        // Indirect reference — dereference and recurse.
        Object::Reference(r) => {
            let inner = doc.get_object(*r).ok()?;
            page_from_dest(doc, inner, oid_to_page)
        }
        // Named destination: a String or Name key into /Names/Dests.
        Object::String(bytes, _) | Object::Name(bytes) => {
            let name_key = bytes.as_slice();
            resolve_named_dest(doc, name_key, oid_to_page)
        }
        _ => None,
    }
}

/// Resolve a named destination through the document's /Names → /Dests tree.
fn resolve_named_dest(
    doc: &Document,
    name: &[u8],
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    // Walk: catalog → /Names dict → /Dests name tree → lookup name → dest array
    let catalog = doc.catalog().ok()?;

    // Try /Dests dictionary directly in catalog first (older PDFs).
    if let Ok(dests_obj) = catalog.get(b"Dests") {
        if let Some(page_idx) = lookup_in_dests(doc, dests_obj, name, oid_to_page) {
            return Some(page_idx);
        }
    }

    // Try /Names → /Dests name tree (modern PDFs).
    let names_obj = catalog.get(b"Names").ok()?;
    let names_dict = deref_dict(doc, names_obj)?;
    let dests_obj = names_dict.get(b"Dests").ok()?;
    lookup_in_name_tree(doc, dests_obj, name, oid_to_page)
}

/// Look up `name` in a name-tree node (which may be a /Kids intermediate node
/// or a leaf with /Names array).
fn lookup_in_name_tree(
    doc: &Document,
    node: &Object,
    name: &[u8],
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    let dict = deref_dict(doc, node)?;

    // Leaf node: /Names array of [key value key value …]
    if let Ok(names_arr) = dict.get(b"Names") {
        if let Ok(arr) = names_arr.as_array() {
            let mut iter = arr.iter();
            while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                let k_bytes = match k {
                    Object::String(b, _) | Object::Name(b) => b.as_slice(),
                    _ => continue,
                };
                if k_bytes == name {
                    return dest_value_to_page(doc, v, oid_to_page);
                }
            }
        }
    }

    // Intermediate node: /Kids array
    if let Ok(kids_obj) = dict.get(b"Kids") {
        if let Ok(kids) = kids_obj.as_array() {
            for kid in kids {
                if let Some(p) = lookup_in_name_tree(doc, kid, name, oid_to_page) {
                    return Some(p);
                }
            }
        }
    }

    None
}

/// Extract the destination page from a value in a /Dests dict or name-tree.
/// The value may be an array or a dict with a /D key.
fn dest_value_to_page(
    doc: &Document,
    val: &Object,
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    match val {
        Object::Reference(r) => {
            if let Ok(inner) = doc.get_object(*r) {
                return dest_value_to_page(doc, inner, oid_to_page);
            }
            None
        }
        Object::Array(_) => page_from_dest(doc, val, oid_to_page),
        Object::Dictionary(d) => {
            let dest = d.get(b"D").ok()?;
            page_from_dest(doc, dest, oid_to_page)
        }
        _ => None,
    }
}

/// Look up a name in the legacy /Dests dict directly under the catalog.
fn lookup_in_dests(
    doc: &Document,
    dests_obj: &Object,
    name: &[u8],
    oid_to_page: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    let dict = deref_dict(doc, dests_obj)?;
    let val = dict.get(name).ok()?;
    dest_value_to_page(doc, val, oid_to_page)
}

/// Dereference an object (follow references) and return it as a Dictionary ref.
fn deref_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(r) => {
            // Can't easily return a reference to an object inside the doc's
            // HashMap without unsafe; use get_object which returns a reference
            // into the document's object store.
            doc.get_object(*r).ok().and_then(|o| o.as_dict().ok())
        }
        _ => None,
    }
}
