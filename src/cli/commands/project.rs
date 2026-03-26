//! Project and source management commands.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::models::{ContentsPageRange, Project, SourceMeta};
use crate::search::SearchIndex;
use crate::storage::{self, cas, Database, ProjectStore};

// ---------------------------------------------------------------------------
// Project listing / creation / display
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
        if let Some(range) = s.contents_page_range {
            println!(
                "    contents_page_range = {} to {}",
                range.start, range.end
            );
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
                println!("    chunks = {chunk_count}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Source management
// ---------------------------------------------------------------------------

pub fn cmd_add(
    project_name: &str,
    path: PathBuf,
    is_textbook: bool,
    start_page: Option<u32>,
    toc_start_page: Option<u32>,
    toc_end_page: Option<u32>,
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

    let contents_page_range = match (toc_start_page, toc_end_page) {
        (Some(start), Some(end)) => {
            if start == 0 || end == 0 {
                anyhow::bail!("--toc-start-page and --toc-end-page must be 1-based and greater than 0");
            }
            if start > end {
                anyhow::bail!("--toc-start-page cannot be greater than --toc-end-page");
            }
            Some(ContentsPageRange { start, end })
        }
        (None, None) => None,
        _ => anyhow::bail!("--toc-start-page and --toc-end-page must be provided together"),
    };

    let mut meta = if is_textbook {
        SourceMeta::textbook(title.clone(), path.clone(), HashMap::new(), start_page)
    } else {
        SourceMeta::report(title.clone(), path.clone())
    };
    meta.contents_page_range = contents_page_range;

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
