//! Structural PDF operations: merge, split, rotate.
//!
//! All operations use `lopdf` for lossless manipulation of the PDF object
//! graph — no re-encoding of page streams.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use lopdf::Document;

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// Merge multiple PDF files into a single output PDF.
///
/// Pages are concatenated in the order provided. The output file is
/// created at `output_path`.
pub fn merge(inputs: &[&Path], output_path: &Path) -> Result<()> {
    if inputs.is_empty() {
        bail!("merge requires at least one input file");
    }
    if inputs.len() == 1 {
        fs::copy(inputs[0], output_path)
            .with_context(|| format!("failed to copy single input to {}", output_path.display()))?;
        return Ok(());
    }

    // Load all documents.
    let mut documents = Vec::new();
    for input in inputs {
        let doc =
            Document::load(input).with_context(|| format!("failed to open {}", input.display()))?;
        documents.push(doc);
    }

    // Build the merged document by extracting pages from each source.
    // Since lopdf doesn't have a built-in merge, we concatenate by
    // cloning the first doc and appending pages from subsequent docs.
    let mut merged = documents.remove(0);

    for doc in &documents {
        let doc_pages = doc.get_pages();
        let total = doc_pages.len() as u32;

        for page_num in 1..=total {
            if let Ok(page_obj) = doc.get_object(doc_pages[&page_num]) {
                // Add page object and its dependencies.
                let new_id = merged.add_object(page_obj.clone());
                // Insert the new page at the end.
                if let Ok(pages_id) = merged
                    .catalog()
                    .and_then(|c| c.get(b"Pages")?.as_reference())
                {
                    if let Ok(pages_obj) = merged.get_object_mut(pages_id) {
                        if let Ok(dict) = pages_obj.as_dict_mut() {
                            if let Ok(kids) = dict.get_mut(b"Kids") {
                                if let Ok(arr) = kids.as_array_mut() {
                                    arr.push(lopdf::Object::Reference(new_id));
                                }
                            }
                            // Update count.
                            if let Ok(count) = dict.get(b"Count").and_then(|o| o.as_i64()) {
                                dict.set("Count", lopdf::Object::Integer(count + 1));
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    merged
        .save(output_path)
        .with_context(|| format!("failed to save merged PDF to {}", output_path.display()))?;

    tracing::info!("Merged {} files → {}", inputs.len(), output_path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Split
// ---------------------------------------------------------------------------

/// Split a PDF into multiple files based on page ranges.
///
/// Each range is `(start, end)` where both are 0-based physical page indices
/// (inclusive). Returns the list of output file paths.
pub fn split(input_path: &Path, ranges: &[(u32, u32)], output_dir: &Path) -> Result<Vec<PathBuf>> {
    if ranges.is_empty() {
        bail!("split requires at least one page range");
    }

    let doc = Document::load(input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;

    let total_pages = doc.get_pages().len() as u32;
    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("split");

    fs::create_dir_all(output_dir)?;

    let mut output_paths = Vec::new();

    for (idx, &(start, end)) in ranges.iter().enumerate() {
        if start >= total_pages || end >= total_pages {
            bail!("page range {start}-{end} exceeds document length {total_pages}");
        }

        let filename = format!("{stem}_pages_{}-{}.pdf", start + 1, end + 1);
        let out_path = output_dir.join(&filename);

        // Clone and remove unwanted pages.
        let mut chunk_doc = doc.clone();
        let first = start + 1; // lopdf is 1-based
        let last = end + 1;
        let pages_to_keep: Vec<u32> = (first..=last).collect();
        let pages_to_delete: Vec<u32> = (1..=total_pages)
            .filter(|p| !pages_to_keep.contains(p))
            .collect();

        chunk_doc.delete_pages(&pages_to_delete);
        chunk_doc
            .save(&out_path)
            .with_context(|| format!("failed to save split chunk {}", out_path.display()))?;

        tracing::info!(
            "Split chunk {}: pages {}-{} → {}",
            idx + 1,
            start,
            end,
            out_path.display()
        );
        output_paths.push(out_path);
    }

    Ok(output_paths)
}

// ---------------------------------------------------------------------------
// Rotate
// ---------------------------------------------------------------------------

/// Rotate specified pages by the given degrees (must be a multiple of 90).
///
/// If `pages` is empty, all pages are rotated. Output is written to
/// `output_path`.
pub fn rotate(input_path: &Path, pages: &[u32], degrees: i32, output_path: &Path) -> Result<()> {
    if degrees % 90 != 0 {
        bail!("rotation degrees must be a multiple of 90, got {degrees}");
    }

    let mut doc = Document::load(input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;

    let page_ids: Vec<(u32, lopdf::ObjectId)> = doc.get_pages().into_iter().collect();

    let target_pages: Vec<u32> = if pages.is_empty() {
        // Rotate all pages.
        page_ids.iter().map(|(num, _)| *num).collect()
    } else {
        // Convert from 0-based to 1-based.
        pages.iter().map(|p| p + 1).collect()
    };

    for (page_num, page_id) in &page_ids {
        if !target_pages.contains(page_num) {
            continue;
        }

        if let Ok(page_obj) = doc.get_object_mut(*page_id) {
            if let Ok(dict) = page_obj.as_dict_mut() {
                let current_rotation = dict
                    .get(b"Rotate")
                    .ok()
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0) as i32;

                let new_rotation = ((current_rotation + degrees) % 360 + 360) % 360;
                dict.set("Rotate", lopdf::Object::Integer(new_rotation as i64));
            }
        }
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    doc.save(output_path)
        .with_context(|| format!("failed to save rotated PDF to {}", output_path.display()))?;

    tracing::info!(
        "Rotated {} page(s) by {degrees}° → {}",
        target_pages.len(),
        output_path.display()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Encrypt
// ---------------------------------------------------------------------------

/// Encrypt a PDF with a user password.
///
/// Uses lopdf's V1 encryption (40-bit RC4, PDF 1.4 compatible).
/// The encrypted file is written to `output_path`.
pub fn encrypt(input_path: &Path, password: &str, output_path: &Path) -> Result<()> {
    if password.is_empty() {
        bail!("password must not be empty");
    }

    let mut doc = Document::load(input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;

    use lopdf::encryption::{EncryptionVersion, Permissions};

    let permissions =
        Permissions::PRINTABLE | Permissions::COPYABLE | Permissions::COPYABLE_FOR_ACCESSIBILITY;

    let version = EncryptionVersion::V1 {
        document: &doc,
        owner_password: password,
        user_password: password,
        permissions,
    };

    let state = lopdf::encryption::EncryptionState::try_from(version)
        .with_context(|| "failed to build encryption state")?;

    doc.encrypt(&state)
        .with_context(|| "failed to encrypt PDF")?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    doc.save(output_path)
        .with_context(|| format!("failed to save encrypted PDF to {}", output_path.display()))?;

    tracing::info!(
        "Encrypted {} → {}",
        input_path.display(),
        output_path.display()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Decrypt
// ---------------------------------------------------------------------------

/// Decrypt a password-protected PDF.
///
/// The decrypted file is written to `output_path`.
pub fn decrypt(input_path: &Path, password: &str, output_path: &Path) -> Result<()> {
    let mut doc = Document::load(input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;

    doc.decrypt(password)
        .with_context(|| "failed to decrypt PDF — wrong password?")?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    doc.save(output_path)
        .with_context(|| format!("failed to save decrypted PDF to {}", output_path.display()))?;

    tracing::info!(
        "Decrypted {} → {}",
        input_path.display(),
        output_path.display()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_rejects_empty_input() {
        let result = merge(&[], Path::new("/tmp/out.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn split_rejects_empty_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let result = split(Path::new("/tmp/test.pdf"), &[], dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_rejects_empty_password() {
        let result = encrypt(Path::new("/tmp/test.pdf"), "", Path::new("/tmp/out.pdf"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn rotate_rejects_non_90() {
        let result = rotate(
            Path::new("/tmp/test.pdf"),
            &[],
            45,
            Path::new("/tmp/out.pdf"),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("multiple of 90"));
    }
}
