//! Arcane — local-first research archival application.
//!
//! # Usage
//!
//! ```text
//! arcane new  "Algorithms"          # create a new project
//! arcane list                        # list all projects
//! arcane add  "Algorithms" /path/to/clrs.pdf --textbook --start-page 12
//! arcane chunk "Algorithms"          # split textbook sources into chapters
//! arcane show "Algorithms"           # show project details
//! ```

mod cli;
mod error;
mod models;
mod pdf;
mod search;
mod storage;
mod ui;
mod watcher;

use clap::Parser;

use cli::commands;
use cli::{Cli, Commands};

fn main() -> anyhow::Result<()> {
    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => commands::cmd_new(&name),
        Commands::List => commands::cmd_list(),
        Commands::Show { name } => commands::cmd_show(&name),
        Commands::Chunk {
            project,
            force,
            depth,
            dry_run,
            source,
        } => commands::cmd_chunk(&project, force, depth, dry_run, source.as_deref()),
        Commands::Add {
            project,
            path,
            textbook,
            start_page,
            title,
            tags,
            source_type,
        } => commands::cmd_add(
            &project,
            path,
            textbook,
            start_page,
            title,
            tags,
            source_type,
        ),
        Commands::ListChunks { project, source } => {
            commands::cmd_list_chunks(&project, source.as_deref())
        }
        Commands::Outline { file, depth } => commands::cmd_outline(file, depth),
        Commands::Remove { project, source } => commands::cmd_remove(&project, source.as_deref()),
        Commands::Merge { output, inputs } => commands::cmd_merge(output, inputs),
        Commands::Split {
            input,
            output_dir,
            ranges,
        } => commands::cmd_split(input, output_dir, ranges),
        Commands::Rotate {
            input,
            degrees,
            output,
            pages,
        } => commands::cmd_rotate(input, degrees, output, pages),
        Commands::Tag { project, tag } => commands::cmd_tag(&project, &tag),
        Commands::Untag { project, tag } => commands::cmd_untag(&project, &tag),
        Commands::Search {
            query,
            limit,
            project,
            source,
        } => commands::cmd_search(&query, limit, project.as_deref(), source.as_deref()),
        Commands::Reindex => commands::cmd_reindex(),
        Commands::Tui => commands::cmd_tui(),
        Commands::Watch { project } => commands::cmd_watch(&project),
        Commands::Probe { file, json } => commands::cmd_probe(file, json),
        Commands::DetectLayout { file, json, pages } => {
            commands::cmd_detect_layout(file, json, pages)
        }
        Commands::FindOffset {
            file,
            toc_pages,
            json,
        } => commands::cmd_find_offset(file, toc_pages, json),
        Commands::Protect {
            input,
            password,
            output,
        } => commands::cmd_protect(input, &password, output),
        Commands::Unlock {
            input,
            password,
            output,
        } => commands::cmd_unlock(input, &password, output),
        Commands::SyncPages {
            file,
            toc_pages,
            threshold,
            json,
        } => commands::cmd_sync_pages(file, toc_pages, threshold, json),
        Commands::Ocr { cmd } => {
            use cli::OcrCommand::*;
            match cmd {
                Init => commands::cmd_ocr_worker_init(),
                Run { file, pages, dpi, json } => commands::cmd_ocr(file, pages, dpi, json),
                Start { idle_timeout_secs } => {
                    commands::cmd_ocr_worker_start(idle_timeout_secs)
                }
                Stop => commands::cmd_ocr_worker_stop(),
                Status => commands::cmd_ocr_worker_status(),
                Restart { idle_timeout_secs } => {
                    commands::cmd_ocr_worker_restart(idle_timeout_secs)
                }
            }
        }
        Commands::InitOcr {
            models_dir,
            skip_runtime,
            force,
        } => commands::cmd_init_ocr(models_dir, skip_runtime, force),
        Commands::WorkerServe { idle_timeout_secs } => {
            commands::cmd_worker_serve(idle_timeout_secs)
        }
    }
}
