use anyhow::{bail, Context};
use lopdf::Document;
use std::path::Path;

/// Extract pages [`start`..=`end`] (1-based physical page numbers) from
/// `input` into a new PDF written to `output`.
pub fn extract_pages(input: &Path, start: u32, end: u32, output: &Path) -> anyhow::Result<()> {
    if start == 0 || start > end {
        bail!("invalid toc-pages range: start={start} end={end}");
    }

    let mut doc = Document::load(input)
        .with_context(|| format!("failed to load PDF: {}", input.display()))?;

    let all_page_nums: Vec<u32> = doc.get_pages().keys().cloned().collect();
    let total = all_page_nums.len() as u32;

    if end > total {
        bail!(
            "toc-pages end={end} exceeds document page count={total} for {}",
            input.display()
        );
    }

    let to_delete: Vec<u32> = all_page_nums
        .iter()
        .filter(|&&p| p < start || p > end)
        .cloned()
        .collect();

    doc.delete_pages(&to_delete);

    doc.save(output)
        .with_context(|| format!("failed to save extracted pages to {}", output.display()))?;

    Ok(())
}
