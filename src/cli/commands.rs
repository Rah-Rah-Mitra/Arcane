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
            let kind = if s.needs_chunking { "textbook" } else { "report" };
            println!(
                "      {} ({}) \u{2014} {}",
                s.title,
                kind,
                s.path.display()
            );
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
        println!(
            "    needs_chunking = {}",
            s.needs_chunking
        );
        if let Some(sp) = s.start_page_physical {
            println!("    start_page_physical = {sp}");
        }
        if !s.chapter_map.is_empty() {
            println!("    chapter_map = {} entries", s.chapter_map.len());
        }
    }
    Ok(())
}

pub fn cmd_chunk(project_name: &str) -> Result<()> {
    let store = ProjectStore::load()?;
    let project = store
        .get(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?;

    let chunks_dir = storage::chunks_dir(project_name)?;

    // Use rayon to chunk sources in parallel.
    let results: Vec<Result<()>> = project.sources
        .par_iter()
        .map(|meta| {
            let source = build_source(meta.clone());
            source.chunk(&chunks_dir)
        })
        .collect();

    // Report any errors.
    for result in results {
        result?;
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

    let chapter_map_json = serde_json::to_string(&meta.chapter_map)
        .unwrap_or_else(|_| "{}".to_string());

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
        let db = Database::open_or_create()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        db.register_blob(
            &blob_ref.hash,
            blob_ref.size,
            &blob_ref.stored_path.to_string_lossy(),
        ).map_err(|e| anyhow::anyhow!("{e}"))?;

        // Create symlink from Originals/ → CAS blob.
        storage::filesystem::link_original_to_cas(
            project_name,
            &blob_ref.stored_path,
            &path,
        )?;

        Some(blob_ref.hash)
    } else {
        println!("[arcane] Warning: file '{}' not found on disk.", path.display());
        None
    };

    // ── Persist to legacy JSON store ─────────────────────────────────────
    {
        let project = store.get_mut(project_name).unwrap();
        project.add_source(meta);
    }
    store.save()?;

    // ── Also persist to SQLite database ──────────────────────────────────
    let db = Database::open_or_create()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Ensure project exists in DB too.
    if !db.project_exists(project_name).map_err(|e| anyhow::anyhow!("{e}"))? {
        db.create_project(project_name).map_err(|e| anyhow::anyhow!("{e}"))?;
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
    ).map_err(|e| anyhow::anyhow!("{e}"))?;

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
    println!(
        "[arcane] Rotated {} → {}",
        input.display(),
        out.display()
    );
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
    let db = Database::open_or_create()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if db.project_exists(project_name).map_err(|e| anyhow::anyhow!("{e}"))? {
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
    let db = Database::open_or_create()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if db.project_exists(project_name).map_err(|e| anyhow::anyhow!("{e}"))? {
        db.remove_project_tag(project_name, tag)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    println!("[arcane] Removed tag '{tag}' from '{project_name}'.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Search commands
// ---------------------------------------------------------------------------

pub fn cmd_search(query: &str, limit: usize) -> Result<()> {
    let idx = SearchIndex::open_or_create()?;
    let results = crate::search::search(&idx, query, limit)?;

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
            let count = idx.index_source(
                &source_id,
                &project.name,
                &source.title,
                None,
                &pages,
            )?;

            total_pages += count;
            total_sources += 1;
        }
    }

    println!(
        "[arcane] Reindexed {total_sources} source(s), {total_pages} page(s) total."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Encryption
// ---------------------------------------------------------------------------

pub fn cmd_protect(input: PathBuf, password: &str, output: Option<PathBuf>) -> Result<()> {
    let out = output.unwrap_or_else(|| input.clone());
    crate::pdf::ops::encrypt(&input, password, &out)?;
    println!(
        "[arcane] Encrypted {} → {}",
        input.display(),
        out.display()
    );
    Ok(())
}

pub fn cmd_unlock(input: PathBuf, password: &str, output: Option<PathBuf>) -> Result<()> {
    let out = output.unwrap_or_else(|| input.clone());
    crate::pdf::ops::decrypt(&input, password, &out)?;
    println!(
        "[arcane] Decrypted {} → {}",
        input.display(),
        out.display()
    );
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

    println!(
        "[arcane] Watching project '{project_name}' for new PDFs. Press Ctrl+C to stop."
    );

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
                        println!(
                            "[arcane]   Stored in CAS (hash: {}…)",
                            &blob_ref.hash[..12]
                        );
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
                let s: u32 = start.parse()
                    .with_context(|| format!("invalid page number in range '{s}'"))?;
                let e: u32 = end.parse()
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
                let p: u32 = single.parse()
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
