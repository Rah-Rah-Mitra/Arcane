//! Chapter detection from PDF `/Outlines` (bookmarks / table of contents).
//!
//! Walks the outline tree's top-level entries and collects each entry's
//! resolved physical page index and title as a chapter boundary.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use lopdf::{Document, Object, ObjectId};

use super::{pdf_string_to_string};

/// Walk the PDF Outlines (bookmarks) tree and collect the first-level entries.
///
/// Each entry contributes one boundary: its resolved physical page index and
/// the bookmark title.  Pages are returned as 0-based physical indices.
pub fn extract_chapters_from_outlines(doc: &Document) -> Result<BTreeMap<u32, String>> {
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
