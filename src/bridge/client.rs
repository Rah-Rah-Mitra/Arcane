use anyhow::{bail, Context};
use reqwest::blocking::multipart;
use serde::Deserialize;
use std::path::Path;

use super::toc::TocEntry;

#[derive(Debug, Deserialize)]
struct TocResponse {
    entries: Vec<TocEntry>,
}

/// Upload a TOC-only PDF to Arcane-PP `/parse-toc` and return parsed entries.
pub fn parse_toc_entries(server_url: &str, pdf_path: &Path) -> anyhow::Result<Vec<TocEntry>> {
    let url = format!("{}/parse-toc", server_url.trim_end_matches('/'));

    let form = multipart::Form::new()
        .file("file", pdf_path)
        .with_context(|| format!("failed to open {} for upload", pdf_path.display()))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .context("failed to build HTTP client")?;

    let resp = client
        .post(&url)
        .multipart(form)
        .send()
        .with_context(|| format!("POST {url} failed"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        bail!("Arcane-PP returned {status}: {body}");
    }

    let parsed: TocResponse = resp
        .json()
        .with_context(|| format!("failed to deserialize response from {url}"))?;

    Ok(parsed.entries)
}
