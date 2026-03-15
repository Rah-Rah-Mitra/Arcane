//! File system watcher — auto-detect new PDFs in project directories.
//!
//! Uses the `notify` crate to watch `~/Arcane/Library/<project>/Originals/`
//! directories. When a new `.pdf` file appears, it is automatically hashed
//! (CAS ingest) and can be auto-indexed.

pub mod handlers;

use std::path::Path;
use std::sync::mpsc;

use anyhow::{Context, Result};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

/// Events emitted by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A new PDF was created or moved into a watched directory.
    NewPdf {
        project_name: String,
        path: std::path::PathBuf,
    },
    /// A file was modified in a watched directory.
    Modified { path: std::path::PathBuf },
    /// A file was removed from a watched directory.
    Removed { path: std::path::PathBuf },
}

/// Watch a project's Originals directory and run the callback for each event.
/// Blocks until an error occurs or the sender is dropped.
pub fn watch_project(project_name: &str, event_tx: mpsc::Sender<WatchEvent>) -> Result<()> {
    let originals = crate::storage::originals_dir(project_name)?;
    watch_directory(&originals, project_name, event_tx)
}

/// Watch a directory and send events through the channel.
pub fn watch_directory(
    dir: &Path,
    project_name: &str,
    event_tx: mpsc::Sender<WatchEvent>,
) -> Result<()> {
    let (tx, rx) = mpsc::channel();

    let mut watcher =
        RecommendedWatcher::new(tx, Config::default()).context("failed to create file watcher")?;

    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch directory: {}", dir.display()))?;

    tracing::info!("Watching {} for project '{project_name}'", dir.display());

    for res in rx {
        match res {
            Ok(event) => {
                let events = handlers::classify_event(&event, project_name);
                for ev in events {
                    if event_tx.send(ev).is_err() {
                        // Receiver dropped — stop watching.
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                tracing::error!("Watch error: {e}");
            }
        }
    }

    Ok(())
}
