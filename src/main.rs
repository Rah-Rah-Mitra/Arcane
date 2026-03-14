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

use cli::{Cli, Commands};
use cli::commands;

fn main() -> anyhow::Result<()> {
    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => commands::cmd_new(&name),
        Commands::List => commands::cmd_list(),
        Commands::Show { name } => commands::cmd_show(&name),
        Commands::Chunk { project } => commands::cmd_chunk(&project),
        Commands::Add {
            project,
            path,
            textbook,
            start_page,
            title,
            tags,
            source_type,
        } => commands::cmd_add(&project, path, textbook, start_page, title, tags, source_type),
        Commands::Merge { output, inputs } => commands::cmd_merge(output, inputs),
        Commands::Split { input, output_dir, ranges } => {
            commands::cmd_split(input, output_dir, ranges)
        }
        Commands::Rotate { input, degrees, output, pages } => {
            commands::cmd_rotate(input, degrees, output, pages)
        }
        Commands::Tag { project, tag } => commands::cmd_tag(&project, &tag),
        Commands::Untag { project, tag } => commands::cmd_untag(&project, &tag),
        Commands::Search { query, limit } => commands::cmd_search(&query, limit),
        Commands::Reindex => commands::cmd_reindex(),
        Commands::Tui => commands::cmd_tui(),
        Commands::Watch { project } => commands::cmd_watch(&project),
        Commands::Protect { input, password, output } => {
            commands::cmd_protect(input, &password, output)
        }
        Commands::Unlock { input, password, output } => {
            commands::cmd_unlock(input, &password, output)
        }
    }
}
