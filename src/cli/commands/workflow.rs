//! Project-level workflow commands.
//!
//! These commands orchestrate multiple base and analysis operations in the
//! context of an Arcane project (source store + filesystem layout).
//!
//! # Workflow compositions
//!
//! ```text
//! chunk   = engine::detect_boundaries + engine::chunk_pdf
//!           + heuristics::inject_outlines (if chapter_map set)
//! reindex = text::extract_all + SearchIndex::index_source  (per source)
//! freq    = SearchIndex + freq::build_frequency_dict
//! search  = SearchIndex::search
//! watch   = watcher::watch_project + cas::ingest (on new PDF)
//! ```

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::models::{build_source, SourceMeta};
use crate::search::SearchIndex;
use crate::storage::{self, ProjectStore};

// ---------------------------------------------------------------------------
// chunk
// ---------------------------------------------------------------------------

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

    // Each source gets its own subdirectory.
    let targets: Vec<(usize, SourceMeta)> = target_indices
        .iter()
        .map(|&i| (i, project.sources[i].clone()))
        .collect();

    let results: Vec<Result<(usize, u32, usize)>> = targets
        .par_iter()
        .map(|(idx, meta)| {
            let chunks_dir = storage::filesystem::source_chunks_dir(project_name, &meta.title)?;

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
    for &idx in &chunked_indices {
        let source = &project.sources[idx];
        if source.chapter_map.is_empty() || !source.path.exists() {
            continue;
        }
        let doc = lopdf::Document::load(&source.path);
        let needs_injection = match &doc {
            Ok(d) => {
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

    if needs_update {
        store.upsert(project);
        store.save()?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Search / index
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
            r.page + 1,
            r.score
        );
    }
    Ok(())
}

pub fn cmd_freq(
    project_name: &str,
    output: Option<std::path::PathBuf>,
    limit: usize,
) -> Result<()> {
    let store = ProjectStore::load()?;
    let _project = store
        .get(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?;

    let idx = SearchIndex::open_or_create()?;
    let entries = crate::search::freq::build_frequency_dict(&idx, project_name)?;

    if entries.is_empty() {
        println!("[arcane] No indexed content for project '{project_name}' — nothing to export.");
        return Ok(());
    }

    let slice = if limit > 0 && limit < entries.len() {
        &entries[..limit]
    } else {
        &entries
    };

    let output_path = match output {
        Some(p) => p,
        None => storage::filesystem::project_dir(project_name)?.join("freq.txt"),
    };

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    crate::search::freq::write_freq_file(slice, &output_path)?;

    println!(
        "[arcane] Wrote {} entries to {}",
        slice.len(),
        output_path.display()
    );
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
// TUI / watcher
// ---------------------------------------------------------------------------

pub fn cmd_tui() -> Result<()> {
    let store = ProjectStore::load()?;
    let projects = store.projects().to_vec();
    crate::ui::run_tui(projects)
}

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
