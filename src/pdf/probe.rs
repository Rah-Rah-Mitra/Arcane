//! PDF classification — determine whether a document is text-based or scanned.
//!
//! Inspects each page's content stream for text-showing operators (`Tj`, `TJ`,
//! `'`, `"`) and image-placing operators (`Do` referencing an XObject with
//! `/Subtype /Image`).  This allows the recovery pipeline to route text-based
//! PDFs through fast heuristic extraction and flag scanned PDFs.

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use super::heuristics::get_page_content_bytes;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Classification of a single PDF page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    /// Page has text-showing operators in its content stream.
    TextBased,
    /// Page has only image XObjects, no text operators.
    ImageOnly,
    /// Page has both text operators and image XObjects.
    Mixed,
    /// Page has no content stream or no recognisable operators.
    Empty,
}

/// Overall document classification derived from per-page results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// All non-empty pages are text-based.
    TextBased,
    /// All non-empty pages are image-only (scanned document).
    Scanned,
    /// Mix of text-based and image-only pages.
    Mixed,
    /// No usable content found on any page.
    Empty,
}

/// Result of probing a PDF document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// Path to the analysed file.
    pub path: String,
    /// Total number of physical pages.
    pub total_pages: u32,
    /// Overall document classification.
    pub document_kind: DocumentKind,
    /// Per-page classification (index = 0-based physical page).
    pub page_kinds: Vec<PageKind>,
    /// Count of text-based pages.
    pub text_page_count: u32,
    /// Count of image-only pages.
    pub image_page_count: u32,
    /// `true` if the document already has an `/Outlines` entry in the catalog.
    pub has_outlines: bool,
    /// `true` if the document has a `/PageLabels` number tree.
    pub has_page_labels: bool,
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a single page by inspecting its content stream.
pub fn classify_page(doc: &Document, page_oid: ObjectId) -> PageKind {
    let bytes = match get_page_content_bytes(doc, page_oid) {
        Some(b) if !b.is_empty() => b,
        _ => return PageKind::Empty,
    };

    let content = match lopdf::content::Content::decode(&bytes) {
        Ok(c) => c,
        Err(_) => return PageKind::Empty,
    };

    let mut has_text = false;
    let mut has_image = false;

    for op in &content.operations {
        match op.operator.as_str() {
            "Tj" | "TJ" | "'" | "\"" => {
                has_text = true;
            }
            "Do" => {
                // Check if the referenced XObject is an image.
                if is_image_xobject(doc, page_oid, op) {
                    has_image = true;
                }
            }
            _ => {}
        }
        // Short-circuit: once we know both, the answer is Mixed.
        if has_text && has_image {
            return PageKind::Mixed;
        }
    }

    match (has_text, has_image) {
        (true, false) => PageKind::TextBased,
        (false, true) => PageKind::ImageOnly,
        (true, true) => PageKind::Mixed,
        (false, false) => PageKind::Empty,
    }
}

/// Probe an entire document: classify every page and compute summary fields.
pub fn probe(doc: &Document, path: &str) -> ProbeResult {
    let pages = doc.get_pages(); // BTreeMap<1-based page_num, ObjectId>
    let total_pages = pages.len() as u32;

    let page_kinds: Vec<PageKind> = pages.values().map(|&oid| classify_page(doc, oid)).collect();

    let text_page_count = page_kinds
        .iter()
        .filter(|k| matches!(k, PageKind::TextBased | PageKind::Mixed))
        .count() as u32;

    let image_page_count = page_kinds
        .iter()
        .filter(|k| matches!(k, PageKind::ImageOnly))
        .count() as u32;

    let document_kind = if total_pages == 0 || page_kinds.iter().all(|k| *k == PageKind::Empty) {
        DocumentKind::Empty
    } else if image_page_count == 0 {
        DocumentKind::TextBased
    } else if text_page_count == 0 {
        DocumentKind::Scanned
    } else {
        DocumentKind::Mixed
    };

    let has_outlines = has_outlines(doc);
    let has_page_labels = has_page_labels(doc);

    ProbeResult {
        path: path.to_string(),
        total_pages,
        document_kind,
        page_kinds,
        text_page_count,
        image_page_count,
        has_outlines,
        has_page_labels,
    }
}

/// Check whether the catalog contains a non-empty `/Outlines` reference.
pub fn has_outlines(doc: &Document) -> bool {
    let catalog = match doc.catalog() {
        Ok(c) => c,
        Err(_) => return false,
    };
    catalog.has(b"Outlines")
}

/// Check whether the catalog contains a `/PageLabels` number tree.
pub fn has_page_labels(doc: &Document) -> bool {
    let catalog = match doc.catalog() {
        Ok(c) => c,
        Err(_) => return false,
    };
    catalog.has(b"PageLabels")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine whether a `Do` operator references an image XObject.
///
/// Looks up the operand name in the page's `/Resources → /XObject` dictionary
/// and checks for `/Subtype /Image`.
fn is_image_xobject(doc: &Document, page_oid: ObjectId, op: &lopdf::content::Operation) -> bool {
    let name = match op.operands.first() {
        Some(Object::Name(n)) => n,
        _ => return false,
    };

    // Resolve the page dictionary.
    let page_dict = match doc.get_object(page_oid).and_then(|o| o.as_dict().cloned()) {
        Ok(d) => d,
        Err(_) => return false,
    };

    // Look up /Resources → /XObject → <name>.
    let resources = match page_dict.get(b"Resources") {
        Ok(r) => r,
        Err(_) => return false,
    };
    let resources_dict = match resolve_dict(doc, resources) {
        Some(d) => d,
        None => return false,
    };
    let xobjects = match resources_dict.get(b"XObject") {
        Ok(x) => x,
        Err(_) => return false,
    };
    let xobjects_dict = match resolve_dict(doc, xobjects) {
        Some(d) => d,
        None => return false,
    };
    let xobj_ref = match xobjects_dict.get(name) {
        Ok(r) => r,
        Err(_) => return false,
    };

    // Resolve the XObject and check /Subtype.
    let xobj = match xobj_ref {
        Object::Reference(r) => match doc.get_object(*r) {
            Ok(o) => o,
            Err(_) => return false,
        },
        other => other,
    };

    match xobj {
        Object::Stream(ref stream) => {
            matches!(
                stream.dict.get(b"Subtype"),
                Ok(Object::Name(ref n)) if n == b"Image"
            )
        }
        _ => false,
    }
}

/// Resolve an `Object` that may be a `Reference` to a `Dictionary`.
fn resolve_dict(doc: &Document, obj: &Object) -> Option<lopdf::Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d.clone()),
        Object::Reference(r) => {
            let resolved = doc.get_object(*r).ok()?;
            resolved.as_dict().ok().cloned()
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Display helpers (for human-readable CLI output)
// ---------------------------------------------------------------------------

impl std::fmt::Display for PageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageKind::TextBased => write!(f, "Text"),
            PageKind::ImageOnly => write!(f, "Image"),
            PageKind::Mixed => write!(f, "Mixed"),
            PageKind::Empty => write!(f, "Empty"),
        }
    }
}

impl std::fmt::Display for DocumentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentKind::TextBased => write!(f, "Text-Based"),
            DocumentKind::Scanned => write!(f, "Scanned (Image-Only)"),
            DocumentKind::Mixed => write!(f, "Mixed (Text + Image)"),
            DocumentKind::Empty => write!(f, "Empty"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal 1-page PDF with custom content-stream bytes.
    fn make_one_page_pdf(content_bytes: &[u8]) -> Document {
        use lopdf::{Dictionary, Stream};

        let mut doc = Document::with_version("1.7");

        // Content stream.
        let stream = Stream::new(Dictionary::new(), content_bytes.to_vec());
        let stream_id = doc.add_object(Object::Stream(stream));

        // Page dictionary.
        let mut page_dict = Dictionary::new();
        page_dict.set("Type", Object::Name(b"Page".to_vec()));
        page_dict.set("Contents", Object::Reference(stream_id));
        // MediaBox is required for a valid page.
        page_dict.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        );
        let page_id = doc.add_object(Object::Dictionary(page_dict));

        // Pages node.
        let mut pages_dict = Dictionary::new();
        pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
        pages_dict.set("Count", Object::Integer(1));
        pages_dict.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        let pages_id = doc.add_object(Object::Dictionary(pages_dict));

        // Patch the page's Parent reference.
        if let Ok(Object::Dictionary(ref mut d)) = doc.get_object_mut(page_id) {
            d.set("Parent", Object::Reference(pages_id));
        }

        // Catalog.
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));

        doc.trailer.set("Root", Object::Reference(catalog_id));

        doc
    }

    #[test]
    fn classify_text_page() {
        // A content stream with a Tj operator → TextBased.
        let content = b"BT /F1 12 Tf (Hello World) Tj ET";
        let doc = make_one_page_pdf(content);
        let pages = doc.get_pages();
        let page_oid = *pages.values().next().unwrap();
        assert_eq!(classify_page(&doc, page_oid), PageKind::TextBased);
    }

    #[test]
    fn classify_empty_page() {
        let doc = make_one_page_pdf(b"");
        let pages = doc.get_pages();
        let page_oid = *pages.values().next().unwrap();
        assert_eq!(classify_page(&doc, page_oid), PageKind::Empty);
    }

    #[test]
    fn probe_text_document() {
        let content = b"BT /F1 12 Tf (Hello) Tj ET";
        let doc = make_one_page_pdf(content);
        let result = probe(&doc, "test.pdf");
        assert_eq!(result.total_pages, 1);
        assert_eq!(result.document_kind, DocumentKind::TextBased);
        assert_eq!(result.text_page_count, 1);
        assert_eq!(result.image_page_count, 0);
        assert!(!result.has_outlines);
        assert!(!result.has_page_labels);
    }

    #[test]
    fn probe_empty_document() {
        let doc = make_one_page_pdf(b"");
        let result = probe(&doc, "empty.pdf");
        assert_eq!(result.document_kind, DocumentKind::Empty);
    }

    #[test]
    fn page_kind_counts_sum_to_total_or_less() {
        // Text + image ≤ total (some pages may be Empty).
        let content = b"BT /F1 12 Tf (Hello) Tj ET";
        let doc = make_one_page_pdf(content);
        let result = probe(&doc, "test.pdf");
        assert!(result.text_page_count + result.image_page_count <= result.total_pages);
    }
}
