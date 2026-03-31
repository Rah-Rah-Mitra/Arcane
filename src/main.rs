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
//!
//! # Base PDF operations (Tier 0)
//! arcane pdf probe book.pdf
//! arcane pdf merge out.pdf a.pdf b.pdf
//! arcane pdf inject-outlines book.pdf --chapters map.json
//!
//! # Analysis (Tier 1)
//! arcane analyze probe book.pdf
//! arcane analyze outline book.pdf --depth 2
//! arcane analyze offset book.pdf --toc-pages "7-18"
//! ```

mod bridge;
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
use cli::{AnalyzeCommands, Cli, Commands, PdfCommands};

fn main() -> anyhow::Result<()> {
    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        // ── Project management ────────────────────────────────────────────
        Commands::New { name } => commands::cmd_new(&name),
        Commands::List => commands::cmd_list(),
        Commands::Show { name } => commands::cmd_show(&name),
        Commands::Add {
            project,
            path,
            textbook,
            start_page,
            toc_start_page,
            toc_end_page,
            title,
            tags,
            source_type,
        } => commands::cmd_add(
            &project,
            path,
            textbook,
            start_page,
            toc_start_page,
            toc_end_page,
            title,
            tags,
            source_type,
        ),
        Commands::Remove { project, source } => commands::cmd_remove(&project, source.as_deref()),
        Commands::Tag { project, tag } => commands::cmd_tag(&project, &tag),
        Commands::Untag { project, tag } => commands::cmd_untag(&project, &tag),
        Commands::ListChunks { project, source } => {
            commands::cmd_list_chunks(&project, source.as_deref())
        }

        // ── Base PDF operations (Tier 0) ──────────────────────────────────
        Commands::Pdf { op } => match op {
            PdfCommands::Merge { output, inputs } => commands::cmd_merge(output, inputs),
            PdfCommands::Split {
                input,
                output_dir,
                ranges,
            } => commands::cmd_split(input, output_dir, ranges),
            PdfCommands::Rotate {
                input,
                degrees,
                output,
                pages,
            } => commands::cmd_rotate(input, degrees, output, pages),
            PdfCommands::Protect {
                input,
                password,
                output,
            } => commands::cmd_protect(input, &password, output),
            PdfCommands::Unlock {
                input,
                password,
                output,
            } => commands::cmd_unlock(input, &password, output),
            PdfCommands::InjectOutlines {
                input,
                chapters,
                output,
            } => commands::cmd_inject_outlines(input, chapters, output),
            PdfCommands::ExtractPages {
                input,
                start,
                end,
                output,
            } => commands::cmd_extract_pages(input, start, end, output),
        },

        // ── Analysis / inspection (Tier 1) ────────────────────────────────
        Commands::Analyze { op } => match op {
            AnalyzeCommands::Probe { file, json } => commands::cmd_probe(file, json),
            AnalyzeCommands::Outline { file, depth } => commands::cmd_outline(file, depth),
            AnalyzeCommands::Layout { file, json, pages } => {
                commands::cmd_detect_layout(file, json, pages)
            }
            AnalyzeCommands::Offset {
                file,
                toc_pages,
                json,
            } => commands::cmd_find_offset(file, toc_pages, json),
            AnalyzeCommands::SyncPages {
                file,
                toc_pages,
                threshold,
                json,
            } => commands::cmd_sync_pages(file, toc_pages, threshold, json),
        },

        // ── Workflow commands (Tier 2) ────────────────────────────────────
        Commands::Chunk {
            project,
            force,
            depth,
            dry_run,
            source,
        } => commands::cmd_chunk(&project, force, depth, dry_run, source.as_deref()),
        Commands::RecoverOutline {
            file,
            output,
            dry_run,
            min_font_ratio,
            depth,
            toc_pages,
            no_inject,
            fuzzy_threshold,
            json,
            seed_pdf,
            seed_file,
            seed_tolerance,
            offset_tolerance,
            toc_start_page,
            toc_end_page,
            page_one,
            anchor,
        } => commands::cmd_recover_outline(
            file,
            output,
            dry_run,
            min_font_ratio,
            depth,
            toc_pages,
            no_inject,
            fuzzy_threshold,
            json,
            seed_pdf,
            seed_file,
            seed_tolerance,
            offset_tolerance,
            toc_start_page,
            toc_end_page,
            page_one,
            anchor,
        ),
        Commands::ProcessToc {
            pdf,
            toc_pages,
            server,
            output,
            depth,
        } => commands::cmd_process_toc(pdf, &toc_pages, &server, output, depth),
        Commands::Recover {
            pdf,
            toc_pages,
            server,
            output,
            depth,
            dry_run,
        } => commands::cmd_recover(pdf, &toc_pages, &server, output, depth, dry_run),
        Commands::RecoverProject {
            project,
            server,
            depth,
            dry_run,
            arcane_data,
        } => commands::cmd_recover_project(&project, &server, depth, dry_run, arcane_data),
        Commands::Search {
            query,
            limit,
            project,
            source,
        } => commands::cmd_search(&query, limit, project.as_deref(), source.as_deref()),
        Commands::Reindex => commands::cmd_reindex(),
        Commands::Freq {
            project,
            output,
            limit,
        } => commands::cmd_freq(&project, output, limit),
        Commands::Tui => commands::cmd_tui(),
        Commands::Watch { project } => commands::cmd_watch(&project),

        // ── Legacy flat commands (hidden, backward compat) ─────────────────
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
        Commands::Probe { file, json } => commands::cmd_probe(file, json),
        Commands::Outline { file, depth } => commands::cmd_outline(file, depth),
        Commands::DetectLayout { file, json, pages } => {
            commands::cmd_detect_layout(file, json, pages)
        }
        Commands::FindOffset {
            file,
            toc_pages,
            json,
        } => commands::cmd_find_offset(file, toc_pages, json),
        Commands::SyncPages {
            file,
            toc_pages,
            threshold,
            json,
        } => commands::cmd_sync_pages(file, toc_pages, threshold, json),
    }
}
