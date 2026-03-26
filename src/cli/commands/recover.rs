//! Outline recovery pipeline commands.
//!
//! These are workflow-level commands that orchestrate multiple base operations
//! (probe → layout → offset → inject) to recover outline bookmarks in PDFs
//! that lack them.
//!
//! # Workflow composition
//!
//! ```text
//! recover-outline = probe + layout::analyze + offset::calculate + heuristics::inject
//! recover         = bridge::extract_pages + bridge::client::parse_toc + recover-outline
//! recover-project = [for each source] recover
//! process-toc     = bridge::extract_pages + bridge::client::parse_toc
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::helpers::{parse_page_range, parse_toc_range_1based, resolve_arcane_data, temp_file_path};

// ---------------------------------------------------------------------------
// recover-outline
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn cmd_recover_outline(
    file: std::path::PathBuf,
    output: Option<std::path::PathBuf>,
    dry_run: bool,
    min_font_ratio: f64,
    depth: u32,
    toc_pages: Option<String>,
    no_inject: bool,
    fuzzy_threshold: f64,
    json: bool,
    seed_pdf: Option<std::path::PathBuf>,
    seed_file: Option<std::path::PathBuf>,
    seed_tolerance: u32,
    offset_tolerance: u32,
    toc_start_page: Option<u32>,
    toc_end_page: Option<u32>,
    page_one: Option<u32>,
    anchor: Vec<(u32, u32)>,
) -> Result<()> {
    use crate::pdf::pipeline::{self, RecoveryConfig};
    use crate::pdf::seed;

    let mut doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    let toc_range = toc_pages.as_deref().and_then(parse_page_range);

    // Load seeds from reference PDF or JSON file (mutually exclusive).
    let seeds = match (seed_pdf, seed_file) {
        (Some(ref_path), None) => {
            let entries = seed::load_seeds_from_pdf(&ref_path, depth)
                .with_context(|| format!("failed to load seeds from {}", ref_path.display()))?;
            println!("[arcane] Loaded {} seed entries from reference PDF.", entries.len());
            Some(entries)
        }
        (None, Some(json_path)) => {
            let entries = seed::load_seeds_from_json(&json_path)
                .with_context(|| format!("failed to load seed file {}", json_path.display()))?;
            println!("[arcane] Loaded {} seed entries from JSON file.", entries.len());
            Some(entries)
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("--seed-pdf and --seed-file are mutually exclusive");
        }
        (None, None) => None,
    };

    // Resolve --toc-start-page / --toc-end-page into toc_range if set.
    let toc_range = match (toc_start_page, toc_end_page, toc_range) {
        (Some(s), Some(e), _) if s >= 1 && e >= s => Some((s - 1, e - 1)),
        (Some(s), None, _) if s >= 1 => Some((s - 1, s - 1)),
        (None, Some(e), _) if e >= 1 => Some((0, e - 1)),
        (_, _, existing) => existing,
    };

    let config = RecoveryConfig {
        min_font_ratio: min_font_ratio as f32,
        max_depth: depth,
        toc_pages: toc_range,
        dry_run,
        inject: !no_inject,
        fuzzy_threshold,
        page_shift_tolerance: seed_tolerance,
        offset_tolerance,
        user_offset: page_one.map(|p| p as i32 - 1),
        user_anchors: anchor,
    };

    let path_str = file.display().to_string();

    let result = if let Some(seed_entries) = seeds {
        pipeline::recover_outline_seeded(&mut doc, &path_str, &config, seed_entries)
            .context("seeded outline recovery pipeline failed")?
    } else {
        pipeline::recover_outline(&mut doc, &path_str, &config)
            .context("outline recovery pipeline failed")?
    };

    // JSON output.
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("failed to serialise recovery result")?
        );

        // Still save if injection happened.
        if result.injected_count.is_some() {
            let out_path = output.unwrap_or_else(|| file.clone());
            doc.save(&out_path)
                .with_context(|| format!("failed to save PDF to {}", out_path.display()))?;
            eprintln!("[arcane] Saved → {}", out_path.display());
        }

        return Ok(());
    }

    // Human-readable output.
    println!("File: {}", file.display());
    println!("Type: {}", result.probe.document_kind);

    if result.chapter_map.is_empty() {
        if result.probe.document_kind == crate::pdf::probe::DocumentKind::Scanned {
            println!("[arcane] Scanned PDF detected — outline recovery is not supported for scanned documents.");
        } else {
            println!(
                "[arcane] No headings detected. Try a lower --min-font-ratio or provide --toc-pages."
            );
        }
        return Ok(());
    }

    // Print offset if available.
    if let Some(ref offset) = result.offset {
        println!(
            "Offset: {:+} (method: {:?}, confidence: {:.0}%)",
            offset.offset,
            offset.method,
            offset.confidence * 100.0
        );
    }

    // Print detected headings.
    println!("\nDetected {} heading(s):\n", result.chapter_map.len());
    println!("  {:<6} {:<8} Title", "Page", "Depth");
    println!("  {}", "\u{2500}".repeat(66));
    for h in &result.headings {
        let depth_label = match h.depth_level {
            1 => "Ch",
            2 => "  Sec",
            3 => "    Sub",
            _ => "      ?",
        };
        println!("  {:<6} {:<8} {}", h.page_index + 1, depth_label, h.text);
    }

    // Print verification summary.
    let verified_count = result.verification.iter().filter(|v| v.verified).count();
    let total = result.verification.len();
    if total > 0 {
        println!(
            "\nVerification: {}/{} headings confirmed on target pages.",
            verified_count, total
        );
    }

    // Print seed verification table (when --seed-pdf / --seed-file was used).
    if let Some(ref seed_ver) = result.seed_verification {
        use crate::pdf::seed::SeedStatus;
        let confirmed = seed_ver
            .iter()
            .filter(|s| s.status == SeedStatus::Confirmed)
            .count();
        let estimated = seed_ver
            .iter()
            .filter(|s| s.status == SeedStatus::Estimated)
            .count();
        let out_of_range = seed_ver
            .iter()
            .filter(|s| s.status == SeedStatus::OutOfRange)
            .count();
        println!(
            "\nSeed verification: {} confirmed, {} estimated, {} out-of-range",
            confirmed, estimated, out_of_range
        );
        println!("  {:<5} {:<6} {}", "Page", "Status", "Title");
        println!("  {}", "\u{2500}".repeat(72));
        for s in seed_ver {
            let flag = match s.status {
                SeedStatus::Confirmed => "OK ",
                SeedStatus::Estimated => "EST",
                SeedStatus::OutOfRange => "OOR",
            };
            println!("  p{:<4} [{}]  {}", s.target_page + 1, flag, s.title);
        }
    }

    if dry_run {
        println!("\n[arcane] Dry-run — no file written.");
        return Ok(());
    }

    // Save if injection happened.
    if let Some(count) = result.injected_count {
        let out_path = output.unwrap_or_else(|| file.clone());
        doc.save(&out_path)
            .with_context(|| format!("failed to save PDF to {}", out_path.display()))?;
        println!(
            "\n[arcane] Injected {count} outline entries → {}",
            out_path.display()
        );
        println!("[arcane] You can now re-chunk with: arcane chunk <project> --source \"<title>\" --force");
    } else if no_inject {
        println!("\n[arcane] --no-inject specified — no file written.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// process-toc  (extract TOC pages and OCR via Arcane-PP)
// ---------------------------------------------------------------------------

pub fn cmd_process_toc(
    pdf: PathBuf,
    toc_pages: &str,
    server: &str,
    output: Option<PathBuf>,
    depth: u32,
) -> Result<()> {
    let entries = extract_toc_entries(&pdf, toc_pages, server, depth)?;
    let json = serde_json::to_string_pretty(&entries).context("failed to serialise seed JSON")?;

    match output {
        Some(path) => {
            std::fs::write(&path, json)
                .with_context(|| format!("failed to write seed JSON to {}", path.display()))?;
            println!("[arcane] Seed JSON written to {}", path.display());
        }
        None => println!("{json}"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// recover  (bridge pipeline: extract TOC + OCR + recover-outline)
// ---------------------------------------------------------------------------

pub fn cmd_recover(
    pdf: PathBuf,
    toc_pages: &str,
    server: &str,
    output: Option<PathBuf>,
    depth: u32,
    dry_run: bool,
) -> Result<()> {
    let entries = extract_toc_entries(&pdf, toc_pages, server, depth)?;

    let temp_seed = temp_file_path("bridge-seed", "json");
    let seed_json = serde_json::to_string_pretty(&entries).context("failed to serialise seed JSON")?;
    std::fs::write(&temp_seed, seed_json).with_context(|| {
        format!(
            "failed to write temporary seed JSON {}",
            temp_seed.display()
        )
    })?;

    let recover_result = cmd_recover_outline(
        pdf,
        output,
        dry_run,
        1.2,
        depth,
        Some(toc_pages.to_string()),
        false,
        0.6,
        false,
        None,
        Some(temp_seed.clone()),
        5,
        50,
        None,
        None,
        None,
        vec![],
    );

    let _ = std::fs::remove_file(&temp_seed);
    recover_result
}

// ---------------------------------------------------------------------------
// recover-project  (batch bridge recovery for all sources in a project)
// ---------------------------------------------------------------------------

pub fn cmd_recover_project(
    project: &str,
    server: &str,
    depth: u32,
    dry_run: bool,
    arcane_data: Option<PathBuf>,
) -> Result<()> {
    let arcane_data = resolve_arcane_data(arcane_data)?;
    let store = crate::bridge::projects::load_projects(&arcane_data)?;
    let sources = crate::bridge::projects::sources_needing_recovery(&store, project);

    if sources.is_empty() {
        println!(
            "[arcane] No sources in project '{project}' need recovery (either chapter_map is populated or TOC page ranges are missing)."
        );
        return Ok(());
    }

    println!(
        "[arcane] Project '{project}': {} source(s) need recovery{}",
        sources.len(),
        if dry_run { " (dry-run)" } else { "" }
    );

    let mut success = 0usize;
    let mut failed = 0usize;

    for source in &sources {
        let toc = source.contents_page_range.as_ref().expect("filtered above");
        let toc_pages = format!("{}-{}", toc.start, toc.end);
        let pdf = source.pdf_path();

        println!("\n[arcane] -- {} --", source.title);
        println!("[arcane]    PDF: {}", pdf.display());
        println!("[arcane]    TOC pages: {toc_pages}");

        match cmd_recover(pdf, &toc_pages, server, None, depth, dry_run) {
            Ok(()) => {
                println!("[arcane]    done");
                success += 1;
            }
            Err(err) => {
                println!("[arcane]    error: {err:#}");
                failed += 1;
            }
        }
    }

    println!("\n[arcane] Finished - {success} succeeded, {failed} failed.");
    if failed > 0 {
        anyhow::bail!("{failed} source(s) failed recovery");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn extract_toc_entries(
    pdf: &Path,
    toc_pages: &str,
    server: &str,
    depth: u32,
) -> Result<Vec<crate::bridge::toc::TocEntry>> {
    let (start, end) = parse_toc_range_1based(toc_pages)?;

    println!(
        "[arcane] Extracting TOC pages {start}-{end} from {} ...",
        pdf.display()
    );
    let temp_toc_pdf = temp_file_path("bridge-toc", "pdf");
    crate::bridge::pdf::extract_pages(pdf, start, end, &temp_toc_pdf)?;

    println!("[arcane] Sending extracted TOC pages to Arcane-PP at {server} ...");
    let parsed = crate::bridge::client::parse_toc_entries(server, &temp_toc_pdf);
    let _ = std::fs::remove_file(&temp_toc_pdf);

    let entries = parsed?;
    if entries.is_empty() {
        anyhow::bail!(
            "No TOC entries could be extracted. Check --toc-pages {toc_pages} and Arcane-PP server {server}."
        );
    }

    let max_depth = entries.iter().map(|e| e.depth).max().unwrap_or(1);
    println!(
        "[arcane] Parsed {} TOC entries (seed max depth {}, requested inject depth {}).",
        entries.len(),
        max_depth,
        depth
    );
    Ok(entries)
}
