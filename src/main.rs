//! Arcane — local-first research archival application.
//!
//! # Usage
//!
//! ```
//! arcane new  "Algorithms"          # create a new project
//! arcane list                        # list all projects
//! arcane add  "Algorithms" /path/to/clrs.pdf --textbook --start-page 12
//! arcane chunk "Algorithms"          # split textbook sources into chapters
//! arcane show "Algorithms"           # show project details
//! ```

mod models;
mod pdf_engine;
mod storage;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use models::{Project, SourceMeta};
use storage::ProjectStore;

// ---------------------------------------------------------------------------
// CLI argument parsing (no external deps — hand-rolled for zero-bloat)
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.as_slice() {
        [] => {
            print_usage();
            Ok(())
        }

        [flag] if flag == "--help" || flag == "-h" => {
            print_usage();
            Ok(())
        }

        [cmd] if cmd == "list" => cmd_list(),

        [cmd, project_name] if cmd == "new" => cmd_new(project_name),

        [cmd, project_name] if cmd == "show" => cmd_show(project_name),

        [cmd, project_name] if cmd == "chunk" => cmd_chunk(project_name),

        // arcane add <project> <path> [--textbook] [--start-page N] [--title T]
        args if args.first().map(String::as_str) == Some("add") => {
            cmd_add(&args[1..])
        }

        other => {
            eprintln!("[arcane] Unknown command: {:?}", other);
            eprintln!("Run `arcane --help` for usage.");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_list() -> Result<()> {
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
        println!("  • {}{}", p.name, tags);
        for s in &p.sources {
            let kind = if s.needs_chunking { "textbook" } else { "report" };
            println!(
                "      {} ({}) — {}",
                s.title,
                kind,
                s.path.display()
            );
        }
    }
    Ok(())
}

fn cmd_new(name: &str) -> Result<()> {
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

fn cmd_show(name: &str) -> Result<()> {
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
        println!("  • {} — {}", s.title, s.path.display());
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

fn cmd_chunk(project_name: &str) -> Result<()> {
    let store = ProjectStore::load()?;
    let project = store
        .get(project_name)
        .with_context(|| format!("project '{project_name}' not found"))?;

    let chunks_dir = storage::chunks_dir(project_name)?;

    for meta in &project.sources {
        let source = models::build_source(meta.clone());
        source.chunk(&chunks_dir)?;
    }
    Ok(())
}

fn cmd_add(rest: &[String]) -> Result<()> {
    // Minimum: project_name  path
    if rest.len() < 2 {
        anyhow::bail!("Usage: arcane add <project> <path> [--textbook] [--start-page N] [--title T]");
    }

    let project_name = &rest[0];
    let path = PathBuf::from(&rest[1]);

    let mut is_textbook = false;
    let mut start_page: Option<u32> = None;
    let mut title_override: Option<String> = None;

    let mut i = 2usize;
    while i < rest.len() {
        match rest[i].as_str() {
            "--textbook" => {
                is_textbook = true;
                i += 1;
            }
            "--start-page" => {
                i += 1;
                let n: u32 = rest
                    .get(i)
                    .context("--start-page requires a value")?
                    .parse()
                    .context("--start-page value must be a non-negative integer")?;
                start_page = Some(n);
                i += 1;
            }
            "--title" => {
                i += 1;
                title_override = Some(
                    rest.get(i)
                        .context("--title requires a value")?
                        .clone(),
                );
                i += 1;
            }
            unknown => {
                anyhow::bail!("Unknown flag: {unknown}");
            }
        }
    }

    let title = title_override.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string()
    });

    let meta = if is_textbook {
        SourceMeta::textbook(title, path.clone(), HashMap::new(), start_page)
    } else {
        SourceMeta::report(title, path.clone())
    };

    let mut store = ProjectStore::load()?;

    // Auto-create project if it doesn't exist.
    if store.get(project_name).is_none() {
        println!("[arcane] Project '{project_name}' not found — creating it.");
        store.upsert(Project::new(project_name));
        storage::originals_dir(project_name)?;
        storage::chunks_dir(project_name)?;
    }

    // Symlink / copy the original PDF.
    if path.exists() {
        let link = storage::link_original(project_name, &path)?;
        println!("[arcane] Original linked at '{}'.", link.display());
    }

    {
        let project = store.get_mut(project_name).unwrap();
        project.add_source(meta);
    }
    store.save()?;
    println!(
        "[arcane] Added '{}' to project '{project_name}'.",
        path.display()
    );
    Ok(())
}

fn print_usage() {
    println!(
        r#"arcane — local-first research archival application

USAGE:
    arcane <COMMAND> [OPTIONS]

COMMANDS:
    new  <project>               Create a new project
    list                         List all projects and their sources
    show <project>               Show details for a project
    add  <project> <pdf-path>    Add a source to a project
         [--textbook]             Mark source as a textbook (needs chunking)
         [--start-page N]         Physical page index where printed Page 1 starts
         [--title T]              Override the source display title
    chunk <project>              Split textbook sources into per-chapter PDFs

FILESYSTEM LAYOUT:
    ~/Arcane/projects.json
    ~/Arcane/Library/<project>/Originals/   (symlinks to original PDFs)
    ~/Arcane/Library/<project>/Chunks/      (01_Chapter.pdf, 02_Chapter.pdf, …)
"#
    );
}
