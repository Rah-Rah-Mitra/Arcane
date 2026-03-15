//! Command handler implementations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::models::{build_source, Project, SourceMeta};
use crate::search::SearchIndex;
use crate::storage::{self, cas, Database, ProjectStore};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn cmd_list() -> Result<()> {
    let store = ProjectStore::load()?;
    if store.projects().is_empty() {
        println!("No projects yet.  Use `arcane new <name>` to create one.");
        return Ok(());
    }
    for p in store.projects() {
        let tags = if p.tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", p.tags.join(", "))
        };
        println!("  \u{2022} {}{}", p.name, tags);
        for s in &p.sources {
            let kind = if s.needs_chunking {
                "textbook"
            } else {
                "report"
            };
            println!("      {} ({}) \u{2014} {}", s.title, kind, s.path.display());
        }
    }
    Ok(())
}

pub fn cmd_new(name: &str) -> Result<()> {
    let mut store = ProjectStore::load()?;
    if store.get(name).is_some() {
        println!("[arcane] Project '{name}' already exists.");
        return Ok(());
    }
    let project = Project::new(name);
    storage::originals_dir(name)?;
    storage::chunks_dir(name)?;
    store.upsert(project);
    store.save()?;
    println!("[arcane] Created project '{name}'.");
    Ok(())
}

pub fn cmd_show(name: &str) -> Result<()> {
    let store = ProjectStore::load()?;
    let project = store
        .get(name)
        .with_context(|| format!("project '{name}' not found"))?;

    println!("Project : {}", project.name);
    if !project.tags.is_empty() {
        println!("Tags    : {}", project.tags.join(", "));
    }
    println!("Sources :");
    if project.sources.is_empty() {
        println!("  (none)");
    }
    for s in &project.sources {
        println!("  \u{2022} {} \u{2014} {}", s.title, s.path.display());
        println!("    needs_chunking = {}", s.needs_chunking);
        if let Some(sp) = s.start_page_physical {
            println!("    start_page_physical = {sp}");
        }
        if !s.chapter_map.is_empty() {
            println!("    chapter_map = {} entries", s.chapter_map.len());
        }
        if let Some(depth) = s.depth {
            println!("    depth = {depth}");
        }
        if let Some(page_count) = s.page_count {
            println!("    page_count = {page_count}");
        }
        // Count chunks if they exist
        if s.needs_chunking {
            let chunks_dir = storage::filesystem::source_chunks_dir(name, &s.title)?;
            if chunks_dir.exists() {
                let chunk_count = std::fs::read_dir(&chunks_dir)?
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|x| x.to_str())
                            .map(|x| x.eq_ignore_ascii_case("pdf"))
                            .unwrap_or(false)
                    })
                    .count();
                if chunk_count > 0 {
                    println!("    chunks = {chunk_count}");
                }
            }
        }
    }
    Ok(())
}

pub fn cmd_chunk(
    project_name: &str,
    force: bool,
    depth: u32,
    dry_run: bool,
    source_filter: Option<&str>,
) -> Result<()> {
    let mut store = ProjectStore::load()?;
    let mut project = store
        .get(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?
        .clone();

    // Determine which source indices to process.
    let target_indices: Vec<usize> = if let Some(title) = source_filter {
        let indices: Vec<usize> = project
            .sources
            .iter()
            .enumerate()
            .filter(|(_, s)| s.title == title)
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            anyhow::bail!("source '{title}' not found in project '{project_name}'");
        }
        indices
    } else {
        (0..project.sources.len()).collect()
    };

    // ── Dry-run: show detected boundaries without writing files ──────────
    if dry_run {
        for &idx in &target_indices {
            let meta = &project.sources[idx];
            if !meta.needs_chunking {
                continue;
            }
            println!("Source: {}", meta.title);
            match crate::pdf::engine::detect_boundaries(meta, depth) {
                Ok(ranges) => {
                    println!("  {:<4} {:<50} Pages", "#", "Chapter");
                    println!("  {}", "\u{2500}".repeat(70));
                    for (i, (start, end, title)) in ranges.iter().enumerate() {
                        println!(
                            "  {:<4} {:<50} {}-{}",
                            format!("{:02}", i + 1),
                            title,
                            start + 1,
                            end + 1
                        );
                    }
                    println!();
                }
                Err(e) => println!("  Error detecting boundaries: {e}\n"),
            }
        }
        return Ok(());
    }

    // Each source gets its own subdirectory so multiple textbooks don't collide
    // and the idempotency guard works correctly per-source.
    let targets: Vec<(usize, SourceMeta)> = target_indices
        .iter()
        .map(|&i| (i, project.sources[i].clone()))
        .collect();

    let results: Vec<Result<(usize, u32, usize)>> = targets
        .par_iter()
        .map(|(idx, meta)| {
            let chunks_dir = storage::filesystem::source_chunks_dir(project_name, &meta.title)?;

            // When --force is set, remove existing chunks so they get regenerated.
            if force && chunks_dir.exists() {
                std::fs::remove_dir_all(&chunks_dir)?;
                std::fs::create_dir_all(&chunks_dir)?;
            }

            let source = build_source(meta.clone());
            let (page_count, chunk_count) = source.chunk(&chunks_dir, depth)?;
            Ok((*idx, page_count, chunk_count))
        })
        .collect();

    // Update metadata for each source that was chunked.
    let mut needs_update = false;
    let mut chunked_indices: Vec<usize> = Vec::new();
    for result in results {
        let (idx, page_count, _chunk_count) = result?;
        let source = &mut project.sources[idx];
        if source.needs_chunking {
            source.depth = Some(depth);
            source.page_count = Some(page_count);
            needs_update = true;
            chunked_indices.push(idx);
        }
    }

    // ── Inject outlines into source PDFs that have a chapter_map ─────────
    // This permanently recovers the outline metadata so future tools can
    // read proper bookmarks from the PDF.
    for &idx in &chunked_indices {
        let source = &project.sources[idx];
        if source.chapter_map.is_empty() || !source.path.exists() {
            continue;
        }
        // Check if the PDF already has outlines — skip if it does.
        let doc = lopdf::Document::load(&source.path);
        let needs_injection = match &doc {
            Ok(d) => {
                // Check for non-empty outlines: the /Outlines dict must have
                // a /First child to be considered populated.
                let has_real_outlines = d
                    .trailer
                    .get(b"Root")
                    .ok()
                    .and_then(|r| r.as_reference().ok())
                    .and_then(|root_id| d.get_object(root_id).ok())
                    .and_then(|obj| obj.as_dict().ok())
                    .and_then(|cat| cat.get(b"Outlines").ok())
                    .and_then(|o| o.as_reference().ok())
                    .and_then(|oid| d.get_object(oid).ok())
                    .and_then(|obj| obj.as_dict().ok())
                    .and_then(|outlines| outlines.get(b"First").ok())
                    .is_some();
                !has_real_outlines
            }
            Err(_) => false,
        };

        if needs_injection {
            if let Ok(mut doc) = doc {
                let chapter_map: std::collections::BTreeMap<u32, String> = source
                    .chapter_map
                    .iter()
                    .map(|(&k, v)| (k, v.clone()))
                    .collect();
                match crate::pdf::heuristics::inject_outlines(&mut doc, &chapter_map) {
                    Ok(n) => {
                        if let Err(e) = doc.save(&source.path) {
                            println!(
                                "[arcane] Warning: failed to save outlines to '{}': {e}",
                                source.path.display()
                            );
                        } else {
                            println!(
                                "[arcane] Injected {n} outline entries into '{}'",
                                source.title
                            );
                        }
                    }
                    Err(e) => {
                        println!(
                            "[arcane] Warning: outline injection failed for '{}': {e}",
                            source.title
                        );
                    }
                }
            }
        }
    }

    // Save the updated project if any metadata changed.
    if needs_update {
        store.upsert(project);
        store.save()?;
    }

    Ok(())
}

pub fn cmd_add(
    project_name: &str,
    path: PathBuf,
    is_textbook: bool,
    start_page: Option<u32>,
    title_override: Option<String>,
    tags: Vec<String>,
    source_type_override: Option<String>,
) -> Result<()> {
    let title = title_override.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string()
    });

    let source_type = if let Some(ref st) = source_type_override {
        st.as_str()
    } else if is_textbook {
        "Textbook"
    } else {
        "Report"
    };

    let meta = if is_textbook {
        SourceMeta::textbook(title.clone(), path.clone(), HashMap::new(), start_page)
    } else {
        SourceMeta::report(title.clone(), path.clone())
    };

    let chapter_map_json =
        serde_json::to_string(&meta.chapter_map).unwrap_or_else(|_| "{}".to_string());

    let mut store = ProjectStore::load()?;

    // Auto-create project if it doesn't exist.
    if store.get(project_name).is_none() {
        println!("[arcane] Project '{project_name}' not found \u{2014} creating it.");
        store.upsert(Project::new(project_name));
        storage::originals_dir(project_name)?;
        storage::chunks_dir(project_name)?;
    }

    // ── CAS ingest: hash and deduplicate ─────────────────────────────────
    let blob_hash = if path.exists() {
        let blob_ref = cas::ingest(&path)?;

        if blob_ref.was_deduplicated {
            println!(
                "[arcane] File already in CAS (hash: {}…) — deduplicated.",
                &blob_ref.hash[..12]
            );
        } else {
            println!(
                "[arcane] Stored in CAS (hash: {}…, {} bytes).",
                &blob_ref.hash[..12],
                blob_ref.size
            );
        }

        // Register blob in database.
        let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;
        db.register_blob(
            &blob_ref.hash,
            blob_ref.size,
            &blob_ref.stored_path.to_string_lossy(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;

        // Create symlink from Originals/ → CAS blob.
        storage::filesystem::link_original_to_cas(project_name, &blob_ref.stored_path, &path)?;

        Some(blob_ref.hash)
    } else {
        println!(
            "[arcane] Warning: file '{}' not found on disk.",
            path.display()
        );
        None
    };

    // ── Persist to legacy JSON store ─────────────────────────────────────
    {
        let project = store.get_mut(project_name).unwrap();
        if !project.add_source(meta) {
            println!(
                "[arcane] Source '{}' is already in project '{project_name}' — skipping.",
                title
            );
            return Ok(());
        }
    }
    store.save()?;

    // ── Also persist to SQLite database ──────────────────────────────────
    let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;

    // Ensure project exists in DB too.
    if !db
        .project_exists(project_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        db.create_project(project_name)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    db.add_source(
        project_name,
        &title,
        &path.to_string_lossy(),
        blob_hash.as_deref(),
        source_type,
        is_textbook,
        start_page,
        &chapter_map_json,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    // ── Apply tags to the project ───────────────────────────────────────
    if !tags.is_empty() {
        // Legacy store.
        {
            let mut store2 = ProjectStore::load()?;
            if let Some(project) = store2.get_mut(project_name) {
                for tag in &tags {
                    if !project.tags.contains(tag) {
                        project.tags.push(tag.clone());
                    }
                }
            }
            store2.save()?;
        }
        // SQLite store.
        for tag in &tags {
            db.add_project_tag(project_name, tag)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        println!("[arcane] Tagged '{project_name}' with: {}", tags.join(", "));
    }

    println!(
        "[arcane] Added '{}' to project '{project_name}'.",
        path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// List chunks command
// ---------------------------------------------------------------------------

pub fn cmd_list_chunks(project_name: &str, source_filter: Option<&str>) -> Result<()> {
    let store = ProjectStore::load()?;
    let project = store
        .get(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?;

    let sources: Vec<&SourceMeta> = if let Some(title) = source_filter {
        project
            .sources
            .iter()
            .filter(|s| s.title == title)
            .collect()
    } else {
        project.sources.iter().collect()
    };

    if sources.is_empty() {
        println!("No matching sources found.");
        return Ok(());
    }

    for meta in &sources {
        let chunks_dir = storage::filesystem::source_chunks_dir(project_name, &meta.title)?;
        println!("Source: {}", meta.title);

        if !chunks_dir.exists() {
            println!("  (no chunks)\n");
            continue;
        }

        let mut entries: Vec<String> = std::fs::read_dir(&chunks_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .collect();

        entries.sort();

        if entries.is_empty() {
            println!("  (no chunks)\n");
            continue;
        }

        for name in &entries {
            println!("  \u{2022} {name}");
        }
        println!("  ({} chunk(s))\n", entries.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recover outline command
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
    ocr: bool,
    ocr_dpi: u32,
    ocr_lang: String,
    ocr_model: Option<String>,
    toc_start_page: Option<u32>,
    toc_end_page: Option<u32>,
    debug_layout: bool,
    page_offset: Option<i32>,
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
    };

    let path_str = file.display().to_string();

    // OCR-only pipeline branch.
    let result = if ocr {
        #[cfg(feature = "ocr")]
        {
            use crate::pdf::pipeline::OcrPipelineConfig;

            let ocr_config = OcrPipelineConfig {
                dpi: ocr_dpi,
                lang: ocr_lang,
                model: ocr_model,
                manual_offset: page_offset,
                debug_layout,
            };

            pipeline::recover_outline_ocr(&mut doc, &path_str, &config, &ocr_config)
                .context("OCR outline recovery pipeline failed")?
        }
        #[cfg(not(feature = "ocr"))]
        {
            // Silence unused variable warnings in the non-ocr build.
            let _ = (ocr_dpi, ocr_lang, ocr_model, debug_layout, page_offset);
            anyhow::bail!(
                "--ocr requires the OCR feature. Rebuild with: cargo build --features ocr"
            );
        }
    } else if let Some(seed_entries) = seeds {
        // Silence unused variable warnings for OCR args in non-OCR path.
        let _ = (ocr_dpi, ocr_lang, ocr_model, debug_layout, page_offset);
        pipeline::recover_outline_seeded(&mut doc, &path_str, &config, seed_entries)
            .context("seeded outline recovery pipeline failed")?
    } else {
        let _ = (ocr_dpi, ocr_lang, ocr_model, debug_layout, page_offset);
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
            println!("[arcane] Scanned PDF detected — OCR support not yet available.");
            println!(
                "         Outline recovery for scanned documents is planned for a future release."
            );
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
// Probe command
// ---------------------------------------------------------------------------

pub fn cmd_probe(file: std::path::PathBuf, json: bool) -> Result<()> {
    use crate::pdf::probe;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    let result = probe::probe(&doc, &file.display().to_string());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("failed to serialise probe result")?
        );
        return Ok(());
    }

    // Human-readable output.
    println!("File:         {}", result.path);
    println!("Pages:        {}", result.total_pages);
    println!("Type:         {}", result.document_kind);
    println!("Text pages:   {}", result.text_page_count);
    println!("Image pages:  {}", result.image_page_count);
    println!(
        "Has outlines: {}",
        if result.has_outlines { "yes" } else { "no" }
    );
    println!(
        "Page labels:  {}",
        if result.has_page_labels { "yes" } else { "no" }
    );

    if result.total_pages <= 30 {
        println!("\nPer-page breakdown:");
        for (i, kind) in result.page_kinds.iter().enumerate() {
            println!("  page {:>4}  {}", i + 1, kind);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Find-offset command
// ---------------------------------------------------------------------------

pub fn cmd_find_offset(
    file: std::path::PathBuf,
    toc_pages: Option<String>,
    json: bool,
) -> Result<()> {
    use crate::pdf::offset;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    // Parse optional --toc-pages "start-end" (1-based) → 0-based inclusive range.
    let toc_range = toc_pages.as_deref().and_then(parse_page_range);

    let result = offset::calculate_offset(&doc, None, toc_range);

    match result {
        Some(ref r) if json => {
            println!(
                "{}",
                serde_json::to_string_pretty(r).context("failed to serialise offset result")?
            );
        }
        Some(ref r) => {
            println!("File:       {}", file.display());
            println!("Offset:     {:+}", r.offset);
            println!("Confidence: {:.0}%", r.confidence * 100.0);
            println!("Method:     {:?}", r.method);
            if !r.evidence.is_empty() {
                println!("\nEvidence:");
                for e in &r.evidence {
                    println!(
                        "  physical page {:>4} → printed page {:>4}  ({})",
                        e.physical_page, e.logical_number, e.matched_text
                    );
                }
            }
        }
        None => {
            if json {
                println!("null");
            } else {
                println!(
                    "[arcane] Could not determine page offset for '{}'.",
                    file.display()
                );
                println!("         Try providing --toc-pages <start>-<end> (1-based).");
            }
        }
    }

    Ok(())
}

/// Parse a "start-end" range string (1-based) into a 0-based inclusive tuple.
fn parse_page_range(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let start: u32 = parts[0].trim().parse().ok()?;
        let end: u32 = parts[1].trim().parse().ok()?;
        if start >= 1 && end >= start {
            return Some((start - 1, end - 1)); // convert to 0-based
        }
    } else if parts.len() == 1 {
        let page: u32 = parts[0].trim().parse().ok()?;
        if page >= 1 {
            return Some((page - 1, page - 1));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Detect-layout command
// ---------------------------------------------------------------------------

pub fn cmd_detect_layout(
    file: std::path::PathBuf,
    json: bool,
    _pages: Option<String>,
) -> Result<()> {
    use crate::pdf::layout;

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    let result = layout::analyze_layout(&doc, &file.display().to_string());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("failed to serialise layout result")?
        );
        return Ok(());
    }

    // Human-readable summary.
    println!("File:           {}", result.path);
    println!("Pages:          {}", result.total_pages);
    println!("Body font size: {:.1}pt", result.body_font_size);
    println!("\nFont clusters:");
    for fc in &result.font_clusters {
        println!(
            "  {:.1}pt  {:>8} chars  {:?}",
            fc.centroid, fc.char_count, fc.role
        );
    }

    if result.anchors.is_empty() {
        println!("\nNo structural anchors detected.");
    } else {
        println!("\nStructural anchors ({}):", result.anchors.len());
        for a in &result.anchors {
            println!(
                "  page {:>4}  y={:>6.1}  {:<18}  {:.0}  {}",
                a.page_index + 1,
                a.y,
                format!("{:?}", a.kind),
                a.font_size,
                a.text
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Outline show command
// ---------------------------------------------------------------------------

pub fn cmd_outline(path: PathBuf, depth: u32) -> Result<()> {
    use crate::pdf::outlines::extract_chapters_with_depth;
    use crate::pdf::page_labels::extract_chapters_from_page_labels;

    let doc = lopdf::Document::load(&path)
        .with_context(|| format!("failed to open PDF at {}", path.display()))?;

    let total_pages = doc.get_pages().len();
    println!("File: {}", path.display());
    println!("Total pages: {total_pages}\n");

    // ── Outlines ─────────────────────────────────────────────────────────
    println!("Outlines (depth {depth}):");
    match extract_chapters_with_depth(&doc, depth) {
        Ok(chapters) => {
            let total = doc.get_pages().len() as u32;
            let ranges = crate::pdf::engine::boundaries_to_ranges(&chapters, total);
            println!("  {:<4} {:<50} Pages", "#", "Title");
            println!("  {}", "\u{2500}".repeat(70));
            for (i, (start, end, title)) in ranges.iter().enumerate() {
                println!(
                    "  {:<4} {:<50} {}-{}",
                    format!("{:02}", i + 1),
                    title,
                    start + 1,
                    end + 1
                );
            }
        }
        Err(e) => println!("  (none — {e})"),
    }

    // ── Page labels ──────────────────────────────────────────────────────
    println!("\nPage labels:");
    match extract_chapters_from_page_labels(&doc) {
        Ok(labels) => {
            for (page_idx, label) in &labels {
                println!("  Page {} — {}", page_idx + 1, label);
            }
        }
        Err(e) => println!("  (none — {e})"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Remove commands
// ---------------------------------------------------------------------------

pub fn cmd_remove(project_name: &str, source_title: Option<&str>) -> Result<()> {
    match source_title {
        Some(title) => cmd_remove_source(project_name, title),
        None => cmd_remove_project(project_name),
    }
}

fn cmd_remove_source(project_name: &str, source_title: &str) -> Result<()> {
    // ── Legacy JSON store ────────────────────────────────────────────────
    let mut store = ProjectStore::load()?;
    let project = store
        .get_mut(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?;

    // Find the source's filename before removing (needed to clean up Originals/).
    let original_filename = project
        .sources
        .iter()
        .find(|s| s.title == source_title)
        .map(|s| {
            s.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

    let before = project.sources.len();
    project.sources.retain(|s| s.title != source_title);
    if project.sources.len() == before {
        anyhow::bail!("source '{source_title}' not found in project '{project_name}'");
    }
    store.save()?;

    // ── SQLite database ──────────────────────────────────────────────────
    let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;
    if db
        .project_exists(project_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        let _ = db
            .delete_source(project_name, source_title)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    // ── Search index ─────────────────────────────────────────────────────
    let source_id = format!("{project_name}:{source_title}");
    let idx = SearchIndex::open_or_create()?;
    let _ = idx.remove_source(&source_id);

    // ── Filesystem cleanup ───────────────────────────────────────────────
    // Remove the symlink/copy in Originals/.
    if let Some(filename) = original_filename {
        let originals = storage::originals_dir(project_name)?;
        let link_path = originals.join(&filename);
        if link_path.exists() {
            std::fs::remove_file(&link_path).ok();
        }
    }

    // Remove the Chunks/{source_title}/ directory.
    let chunks = storage::filesystem::source_chunks_dir(project_name, source_title)?;
    if chunks.exists() {
        std::fs::remove_dir_all(&chunks).ok();
    }

    println!("[arcane] Removed source '{source_title}' from project '{project_name}'.");
    Ok(())
}

fn cmd_remove_project(project_name: &str) -> Result<()> {
    // ── Collect source titles first (for search index cleanup) ───────────
    let store = ProjectStore::load()?;
    let source_titles: Vec<String> = store
        .get(project_name)
        .map(|p| p.sources.iter().map(|s| s.title.clone()).collect())
        .unwrap_or_default();

    // ── Legacy JSON store ────────────────────────────────────────────────
    let mut store = ProjectStore::load()?;
    if !store.remove(project_name) {
        anyhow::bail!("project '{project_name}' not found");
    }
    store.save()?;

    // ── SQLite database ──────────────────────────────────────────────────
    let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;
    let _ = db
        .delete_project(project_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // ── Search index (remove all sources) ────────────────────────────────
    let idx = SearchIndex::open_or_create()?;
    for title in &source_titles {
        let source_id = format!("{project_name}:{title}");
        let _ = idx.remove_source(&source_id);
    }

    // ── Filesystem cleanup ───────────────────────────────────────────────
    let project_path = storage::filesystem::project_dir(project_name)?;
    if project_path.exists() {
        std::fs::remove_dir_all(&project_path).ok();
    }

    println!("[arcane] Removed project '{project_name}' and all its sources.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Structural PDF operations
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
// Tag commands
// ---------------------------------------------------------------------------

pub fn cmd_tag(project_name: &str, tag: &str) -> Result<()> {
    // Legacy JSON store.
    let mut store = ProjectStore::load()?;
    if let Some(project) = store.get_mut(project_name) {
        if !project.tags.contains(&tag.to_string()) {
            project.tags.push(tag.to_string());
        }
        store.save()?;
    } else {
        anyhow::bail!("project '{project_name}' not found");
    }

    // SQLite store.
    let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;
    if db
        .project_exists(project_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        db.add_project_tag(project_name, tag)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    println!("[arcane] Tagged '{project_name}' with '{tag}'.");
    Ok(())
}

pub fn cmd_untag(project_name: &str, tag: &str) -> Result<()> {
    // Legacy JSON store.
    let mut store = ProjectStore::load()?;
    if let Some(project) = store.get_mut(project_name) {
        project.tags.retain(|t| t != tag);
        store.save()?;
    } else {
        anyhow::bail!("project '{project_name}' not found");
    }

    // SQLite store.
    let db = Database::open_or_create().map_err(|e| anyhow::anyhow!("{e}"))?;
    if db
        .project_exists(project_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        db.remove_project_tag(project_name, tag)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    println!("[arcane] Removed tag '{tag}' from '{project_name}'.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Search commands
// ---------------------------------------------------------------------------

pub fn cmd_search(
    query: &str,
    limit: usize,
    project_filter: Option<&str>,
    source_filter: Option<&str>,
) -> Result<()> {
    let idx = SearchIndex::open_or_create()?;
    let results = crate::search::search(&idx, query, limit, project_filter, source_filter)?;

    if results.is_empty() {
        println!("No results found for '{query}'.");
        return Ok(());
    }

    println!("Found {} result(s) for '{query}':\n", results.len());
    for (i, r) in results.iter().enumerate() {
        println!(
            "  {}. [{}] {} — ch. \"{}\" (page {}, score {:.3})",
            i + 1,
            r.project_name,
            r.source_title,
            r.chapter_title,
            r.page + 1, // display as 1-based
            r.score
        );
    }
    Ok(())
}

pub fn cmd_reindex() -> Result<()> {
    let store = ProjectStore::load()?;
    let idx = SearchIndex::open_or_create()?;

    let mut total_pages = 0u64;
    let mut total_sources = 0u64;

    for project in store.projects() {
        for source in &project.sources {
            if !source.path.exists() {
                tracing::warn!(
                    "Skipping '{}' — file not found at {}",
                    source.title,
                    source.path.display()
                );
                continue;
            }

            let pages = crate::pdf::text::extract_all(&source.path)?;
            let source_id = format!("{}:{}", project.name, source.title);

            // Remove old entries for this source, then re-index.
            let _ = idx.remove_source(&source_id);
            let count = idx.index_source(&source_id, &project.name, &source.title, None, &pages)?;

            total_pages += count;
            total_sources += 1;
        }
    }

    println!("[arcane] Reindexed {total_sources} source(s), {total_pages} page(s) total.");
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
// TUI
// ---------------------------------------------------------------------------

pub fn cmd_tui() -> Result<()> {
    let store = ProjectStore::load()?;
    let projects = store.projects().to_vec();
    crate::ui::run_tui(projects)
}

// ---------------------------------------------------------------------------
// File watcher
// ---------------------------------------------------------------------------

pub fn cmd_watch(project_name: &str) -> Result<()> {
    let store = ProjectStore::load()?;
    if store.get(project_name).is_none() {
        anyhow::bail!("project '{project_name}' not found");
    }

    println!("[arcane] Watching project '{project_name}' for new PDFs. Press Ctrl+C to stop.");

    let (tx, rx) = std::sync::mpsc::channel();

    let name = project_name.to_string();
    std::thread::spawn(move || {
        if let Err(e) = crate::watcher::watch_project(&name, tx) {
            tracing::error!("Watcher error: {e}");
        }
    });

    for event in rx {
        match event {
            crate::watcher::WatchEvent::NewPdf { project_name, path } => {
                println!(
                    "[arcane] New PDF detected: {} in project '{project_name}'",
                    path.display()
                );

                // Auto-ingest into CAS.
                match crate::storage::cas::ingest(&path) {
                    Ok(blob_ref) => {
                        println!("[arcane]   Stored in CAS (hash: {}…)", &blob_ref.hash[..12]);
                    }
                    Err(e) => {
                        println!("[arcane]   CAS ingest failed: {e}");
                    }
                }
            }
            crate::watcher::WatchEvent::Modified { path, .. } => {
                println!("[arcane] Modified: {}", path.display());
            }
            crate::watcher::WatchEvent::Removed { path, .. } => {
                println!("[arcane] Removed: {}", path.display());
            }
        }
    }

    Ok(())
}

/// Parse page range strings like "1-5", "6-10" into `(start, end)` tuples
/// (converting from 1-based user input to 0-based internal representation).
fn parse_page_ranges(range_strs: &[String]) -> Result<Vec<(u32, u32)>> {
    let mut ranges = Vec::new();
    for s in range_strs {
        let parts: Vec<&str> = s.split('-').collect();
        match parts.as_slice() {
            [start, end] => {
                let s: u32 = start
                    .parse()
                    .with_context(|| format!("invalid page number in range '{s}'"))?;
                let e: u32 = end
                    .parse()
                    .with_context(|| format!("invalid page number in range '{s}'"))?;
                if s == 0 || e == 0 {
                    anyhow::bail!("page numbers must be >= 1 in range '{s}'");
                }
                if s > e {
                    anyhow::bail!("invalid range '{s}': start > end");
                }
                // Convert to 0-based.
                ranges.push((s - 1, e - 1));
            }
            [single] => {
                let p: u32 = single
                    .parse()
                    .with_context(|| format!("invalid page number '{s}'"))?;
                if p == 0 {
                    anyhow::bail!("page numbers must be >= 1");
                }
                ranges.push((p - 1, p - 1));
            }
            _ => anyhow::bail!("invalid range format: '{s}'. Expected 'N' or 'N-M'."),
        }
    }
    Ok(ranges)
}

// ---------------------------------------------------------------------------
// sync-pages command
// ---------------------------------------------------------------------------

/// One matched heading ↔ TOC-entry pair.
#[derive(Debug, serde::Serialize)]
pub struct SyncMatch {
    pub toc_title: String,
    pub printed_page: u32,
    pub physical_page: u32,
    pub similarity: f64,
    pub delta: i32,
    pub is_inlier: bool,
}

/// Result of `sync-pages` consensus offset estimation.
#[derive(Debug, serde::Serialize)]
pub struct PageSyncResult {
    pub consensus_offset: i32,
    /// Fraction of candidates consistent with the consensus offset.
    pub confidence: f32,
    pub total_candidates: usize,
    pub inlier_count: usize,
    pub matches: Vec<SyncMatch>,
}

pub fn cmd_sync_pages(
    file: std::path::PathBuf,
    toc_pages: Option<String>,
    threshold: f64,
    json: bool,
) -> Result<()> {
    use std::collections::HashMap;

    use crate::pdf::{layout, offset};

    let doc = lopdf::Document::load(&file)
        .with_context(|| format!("failed to open PDF at {}", file.display()))?;

    // Run full layout analysis (feature-vector pipeline).
    let layout_result = layout::analyze_layout(&doc, &file.display().to_string());

    // Determine TOC pages.
    let toc_page_range: Option<(u32, u32)> = if let Some(ref s) = toc_pages {
        parse_page_range(s)
    } else {
        None
    };

    // Extract positioned text for TOC parsing.
    let positioned = layout::extract_all_positioned(&doc);

    // Detect or use supplied TOC pages.
    let effective_toc_range: Option<(u32, u32)> = if let Some(range) = toc_page_range {
        Some(range)
    } else {
        let auto = layout::detect_toc_pages(&positioned);
        if auto.is_empty() {
            None
        } else {
            let &min_p = auto.iter().min().unwrap();
            let &max_p = auto.iter().max().unwrap();
            Some((min_p, max_p))
        }
    };

    // Parse TOC entries: (title, printed_page).
    let toc_entries: Vec<(String, u32)> = if let Some(range) = effective_toc_range {
        offset::parse_toc_entries(&positioned, range)
    } else {
        vec![]
    };

    if toc_entries.is_empty() {
        if json {
            let result = PageSyncResult {
                consensus_offset: 0,
                confidence: 0.0,
                total_candidates: 0,
                inlier_count: 0,
                matches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("No TOC entries found. Try --toc-pages <range>.");
        }
        return Ok(());
    }

    // Collect chapter/section heading anchors.
    let heading_anchors: Vec<&layout::LayoutAnchor> = layout_result
        .anchors
        .iter()
        .filter(|a| {
            matches!(
                a.kind,
                layout::AnchorKind::ChapterHeading
                    | layout::AnchorKind::SectionHeading
                    | layout::AnchorKind::NumberedHeading
            )
        })
        .collect();

    // RANSAC offset estimation.
    // For every heading × TOC-entry pair above threshold, compute delta.
    let mut candidates: Vec<SyncMatch> = Vec::new();
    let mut delta_votes: HashMap<i32, f64> = HashMap::new(); // delta → total_similarity

    for (toc_title, printed_page) in &toc_entries {
        for anchor in &heading_anchors {
            let sim = strsim::normalized_levenshtein(anchor.text.trim(), toc_title.trim());
            if sim >= threshold {
                let delta = anchor.page_index as i32 - *printed_page as i32;
                *delta_votes.entry(delta).or_insert(0.0) += sim;
                candidates.push(SyncMatch {
                    toc_title: toc_title.clone(),
                    printed_page: *printed_page,
                    physical_page: anchor.page_index,
                    similarity: sim,
                    delta,
                    is_inlier: false, // filled in below
                });
            }
        }
    }

    if candidates.is_empty() {
        if json {
            let result = PageSyncResult {
                consensus_offset: 0,
                confidence: 0.0,
                total_candidates: 0,
                inlier_count: 0,
                matches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "No heading↔TOC matches above threshold {threshold:.2}. Try a lower --threshold."
            );
        }
        return Ok(());
    }

    // Consensus = delta with the highest total similarity score.
    let consensus_offset = delta_votes
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(&delta, _)| delta)
        .unwrap_or(0);

    // Mark inliers (|delta - consensus| ≤ 1).
    for m in &mut candidates {
        m.is_inlier = (m.delta - consensus_offset).abs() <= 1;
    }
    let inlier_count = candidates.iter().filter(|m| m.is_inlier).count();
    let confidence = inlier_count as f32 / candidates.len() as f32;

    let result = PageSyncResult {
        consensus_offset,
        confidence,
        total_candidates: candidates.len(),
        inlier_count,
        matches: candidates,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    // Human-readable output.
    println!("File:             {}", file.display());
    println!("Consensus offset: {}", result.consensus_offset);
    println!(
        "Confidence:       {:.0}%  ({} / {} inliers)",
        result.confidence * 100.0,
        result.inlier_count,
        result.total_candidates
    );
    println!();
    println!(
        "{:<40}  {:>5}  {:>8}  {:>6}  Inlier",
        "TOC Title", "Print", "Physical", "Sim"
    );
    println!("{}", "-".repeat(78));
    for m in &result.matches {
        println!(
            "{:<40}  {:>5}  {:>8}  {:>5.2}  {}",
            &m.toc_title[..m.toc_title.len().min(40)],
            m.printed_page,
            m.physical_page + 1,
            m.similarity,
            if m.is_inlier { "✓" } else { "✗" }
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// ocr — run OCR on a page range and output the recognised text
// ---------------------------------------------------------------------------

pub fn cmd_ocr(
    file: PathBuf,
    pages: String,
    dpi: u32,
    json: bool,
) -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        let _ = (&file, &pages, dpi, json);
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr\n\
             Then run: arcane init-ocr"
        );
    }

    #[cfg(feature = "ocr")]
    {
        use crate::pdf::ocr;

        let (start, end) = parse_page_range(&pages)
            .context("invalid --pages range (expected e.g. \"1-5\" or \"14-20\")")?;

        // Convert from 1-based inclusive to 0-based page indices.
        let page_indices: Vec<u32> = (start..=end).collect();

        let results =
            ocr::extract_text_ocr(&file, &page_indices, dpi).context("OCR extraction failed")?;

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&results)
                    .context("failed to serialise OCR results")?
            );
        } else {
            for page in &results {
                let display_page = page.page_index + 1;
                println!("--- Page {} ---", display_page);
                println!("{}", page.full_text());
                println!();
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// init-ocr — download OCR models and runtime libraries
// ---------------------------------------------------------------------------

pub fn cmd_init_ocr(
    models_dir_override: Option<PathBuf>,
    skip_runtime: bool,
    force: bool,
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    let dest_dir = match models_dir_override {
        Some(dir) => {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
            dir
        }
        None => storage::models_dir()?,
    };

    println!("[arcane] OCR setup — downloading to {}", dest_dir.display());
    println!();

    // Build platform-specific manifest.
    let items = build_download_manifest(skip_runtime)?;

    let bar_style =
        ProgressStyle::with_template("  {prefix:<30} [{bar:25}] {bytes}/{total_bytes}  {msg}")
            .unwrap()
            .progress_chars("=> ");

    for item in &items {
        let target = dest_dir.join(item.filename);
        if target.exists() && !force {
            println!("  [skip] {} — already exists", item.filename);
            continue;
        }

        let pb = ProgressBar::new(item.size_hint_bytes);
        pb.set_style(bar_style.clone());
        pb.set_prefix(item.label.to_string());

        // Download to .part file first, then rename/extract.
        let part_path = dest_dir.join(format!("{}.part", item.filename));
        download_to_file(item.url, &part_path, &pb)?;

        if let Some((inner_path, archive_kind)) = &item.extract {
            // Extract the target file from the archive.
            match archive_kind {
                ArchiveKind::Zip => extract_from_zip(&part_path, inner_path, &target)?,
                ArchiveKind::TarGz => extract_from_tgz(&part_path, inner_path, &target)?,
            }
            let _ = std::fs::remove_file(&part_path);
        } else {
            std::fs::rename(&part_path, &target).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    part_path.display(),
                    target.display()
                )
            })?;
        }

        pb.finish_with_message("done");
    }

    println!();
    println!(
        "[arcane] OCR setup complete. Files in {}",
        dest_dir.display()
    );
    println!();
    println!("Build with OCR support:");
    println!("  cargo build --release --features ocr");
    println!();
    println!("Models will be auto-detected at runtime. To override paths:");
    println!("  ARCANE_OCR_DET_MODEL, ARCANE_OCR_REC_MODEL, ARCANE_OCR_DICT");
    println!("  ORT_DYLIB_PATH (ONNX Runtime)");

    Ok(())
}

// ---------------------------------------------------------------------------
// init-ocr helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ArchiveKind {
    Zip,
    TarGz,
}

struct DownloadItem {
    label: &'static str,
    url: &'static str,
    filename: &'static str,
    size_hint_bytes: u64,
    extract: Option<(&'static str, ArchiveKind)>,
}

fn build_download_manifest(skip_runtime: bool) -> Result<Vec<DownloadItem>> {
    let mut items = vec![
        DownloadItem {
            label: "Detection model",
            url: "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-ocrv5_mobile_det.onnx",
            filename: "pp-ocrv5_mobile_det.onnx",
            size_hint_bytes: 4_800_000,
            extract: None,
        },
        DownloadItem {
            label: "Recognition model (English)",
            url: "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/en_pp-ocrv5_mobile_rec.onnx",
            filename: "en_pp-ocrv5_mobile_rec.onnx",
            size_hint_bytes: 7_800_000,
            extract: None,
        },
        DownloadItem {
            label: "Dictionary (English)",
            url: "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/ppocrv5_en_dict.txt",
            filename: "ppocrv5_en_dict.txt",
            size_hint_bytes: 1_500,
            extract: None,
        },
    ];

    if !skip_runtime {
        let (ort_item, pdfium_item) = platform_runtime_items()?;
        items.push(ort_item);
        items.push(pdfium_item);
    }

    Ok(items)
}

fn platform_runtime_items() -> Result<(DownloadItem, DownloadItem)> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);

    let ort = match (os, arch) {
        ("windows", "x86_64") => DownloadItem {
            label: "ONNX Runtime",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.1/onnxruntime-win-x64-1.24.1.zip",
            filename: "onnxruntime.dll",
            size_hint_bytes: 14_000_000,
            extract: Some((
                "onnxruntime-win-x64-1.24.1/lib/onnxruntime.dll",
                ArchiveKind::Zip,
            )),
        },
        ("linux", "x86_64") => DownloadItem {
            label: "ONNX Runtime",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.1/onnxruntime-linux-x64-1.24.1.tgz",
            filename: "libonnxruntime.so",
            size_hint_bytes: 14_000_000,
            extract: Some((
                "onnxruntime-linux-x64-1.24.1/lib/libonnxruntime.so.1.24.1",
                ArchiveKind::TarGz,
            )),
        },
        ("linux", "aarch64") => DownloadItem {
            label: "ONNX Runtime",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.1/onnxruntime-linux-aarch64-1.24.1.tgz",
            filename: "libonnxruntime.so",
            size_hint_bytes: 12_000_000,
            extract: Some((
                "onnxruntime-linux-aarch64-1.24.1/lib/libonnxruntime.so.1.24.1",
                ArchiveKind::TarGz,
            )),
        },
        ("macos", "aarch64") => DownloadItem {
            label: "ONNX Runtime",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.1/onnxruntime-osx-arm64-1.24.1.tgz",
            filename: "libonnxruntime.dylib",
            size_hint_bytes: 8_000_000,
            extract: Some((
                "onnxruntime-osx-arm64-1.24.1/lib/libonnxruntime.1.24.1.dylib",
                ArchiveKind::TarGz,
            )),
        },
        ("macos", "x86_64") => DownloadItem {
            label: "ONNX Runtime (arm64/Rosetta)",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.1/onnxruntime-osx-arm64-1.24.1.tgz",
            filename: "libonnxruntime.dylib",
            size_hint_bytes: 8_000_000,
            extract: Some((
                "onnxruntime-osx-arm64-1.24.1/lib/libonnxruntime.1.24.1.dylib",
                ArchiveKind::TarGz,
            )),
        },
        _ => anyhow::bail!(
            "Unsupported platform: {os}/{arch}. Use --skip-runtime and download manually."
        ),
    };

    let pdfium = match (os, arch) {
        ("windows", "x86_64") => DownloadItem {
            label: "PDFium",
            url: "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-win-x64.tgz",
            filename: "pdfium.dll",
            size_hint_bytes: 7_000_000,
            extract: Some(("bin/pdfium.dll", ArchiveKind::TarGz)),
        },
        ("linux", "x86_64") => DownloadItem {
            label: "PDFium",
            url: "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz",
            filename: "libpdfium.so",
            size_hint_bytes: 7_000_000,
            extract: Some(("lib/libpdfium.so", ArchiveKind::TarGz)),
        },
        ("linux", "aarch64") => DownloadItem {
            label: "PDFium",
            url: "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-arm64.tgz",
            filename: "libpdfium.so",
            size_hint_bytes: 7_000_000,
            extract: Some(("lib/libpdfium.so", ArchiveKind::TarGz)),
        },
        ("macos", "aarch64") => DownloadItem {
            label: "PDFium",
            url: "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-arm64.tgz",
            filename: "libpdfium.dylib",
            size_hint_bytes: 7_000_000,
            extract: Some(("lib/libpdfium.dylib", ArchiveKind::TarGz)),
        },
        ("macos", "x86_64") => DownloadItem {
            label: "PDFium",
            url: "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-x64.tgz",
            filename: "libpdfium.dylib",
            size_hint_bytes: 7_000_000,
            extract: Some(("lib/libpdfium.dylib", ArchiveKind::TarGz)),
        },
        _ => anyhow::bail!(
            "Unsupported platform: {os}/{arch}. Use --skip-runtime and download manually."
        ),
    };

    Ok((ort, pdfium))
}

fn download_to_file(url: &str, dest: &std::path::Path, pb: &indicatif::ProgressBar) -> Result<()> {
    use std::io::{Read as _, Write as _};

    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("failed to download {url}"))?;

    let len: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    if let Some(len) = len {
        pb.set_length(len);
    }

    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest)
        .with_context(|| format!("failed to create {}", dest.display()))?;
    let mut buf = [0u8; 32768];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        pb.inc(n as u64);
    }

    Ok(())
}

fn extract_from_zip(archive: &std::path::Path, inner: &str, dest: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut entry = zip
        .by_name(inner)
        .with_context(|| format!("{inner} not found in archive"))?;
    let mut out = std::fs::File::create(dest)?;
    std::io::copy(&mut entry, &mut out)?;
    Ok(())
}

fn extract_from_tgz(archive: &std::path::Path, inner: &str, dest: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().replace('\\', "/");
        if path == inner
            || path.ends_with(&format!("/{}", inner.rsplit('/').next().unwrap_or(inner)))
        {
            let mut out = std::fs::File::create(dest)?;
            std::io::copy(&mut entry, &mut out)?;
            return Ok(());
        }
    }
    anyhow::bail!("{inner} not found in archive {}", archive.display())
}

// ---------------------------------------------------------------------------
// ocr worker commands
// ---------------------------------------------------------------------------

pub fn cmd_ocr_worker_init() -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr\n\
             Then run: arcane init-ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        if crate::pdf::worker::try_connect().is_some() {
            println!("[arcane] OCR worker is already running and initialized.");
            return Ok(());
        }

        println!("[arcane] Initializing OCR worker (warm-up)…");
        crate::pdf::worker::cmd_start(None)?;
        crate::pdf::worker::cmd_stop()?;
        println!("[arcane] OCR initialization complete.");
        Ok(())
    }
}

pub fn cmd_ocr_worker_start(idle_timeout_secs: Option<u64>) -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        let _ = idle_timeout_secs;
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr\n\
             Then run: arcane init-ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        crate::pdf::worker::cmd_start(idle_timeout_secs)
    }
}

pub fn cmd_ocr_worker_stop() -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        crate::pdf::worker::cmd_stop()
    }
}

pub fn cmd_ocr_worker_status() -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        crate::pdf::worker::cmd_status()
    }
}

pub fn cmd_ocr_worker_restart(idle_timeout_secs: Option<u64>) -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        let _ = idle_timeout_secs;
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        crate::pdf::worker::cmd_restart(idle_timeout_secs)
    }
}

pub fn cmd_worker_serve(idle_timeout_secs: Option<u64>) -> Result<()> {
    #[cfg(not(feature = "ocr"))]
    {
        let _ = idle_timeout_secs;
        anyhow::bail!(
            "OCR support is not compiled in.\n\
             Rebuild with: cargo build --release --features ocr"
        );
    }
    #[cfg(feature = "ocr")]
    {
        crate::pdf::worker::serve_loop(idle_timeout_secs)
    }
}
