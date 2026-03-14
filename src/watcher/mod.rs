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
    Modified {
        project_name: String,
        path: std::path::PathBuf,
    },
    /// A file was removed from a watched directory.
    Removed {
        project_name: String,
        path: std::path::PathBuf,
    },
}

/// Watch a project's Originals directory and run the callback for each event.
/// Blocks until an error occurs or the sender is dropped.
pub fn watch_project(
    project_name: &str,
    event_tx: mpsc::Sender<WatchEvent>,
) -> Result<()> {
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

    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .context("failed to create file watcher")?;

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    #[ignore] // Platform-dependent integration test — run with `cargo test -- --ignored`
    fn watcher_detects_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let dir_path = dir.path().to_path_buf();

        let handle = std::thread::spawn(move || {
            let _ = watch_directory(&dir_path, "TestProject", tx);
        });

        // Give the watcher time to start.
        std::thread::sleep(Duration::from_millis(200));

        // Create a PDF file.
        let pdf_path = dir.path().join("test.pdf");
        fs::write(&pdf_path, b"fake pdf content").unwrap();

        // Wait for events (with timeout).
        // On Windows, file creation may emit Create or Modify events.
        let mut found_pdf_event = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
                match &event {
                    WatchEvent::NewPdf { project_name, path }
                    | WatchEvent::Modified { project_name, path } => {
                        assert_eq!(project_name, "TestProject");
                        if path.to_string_lossy().contains("test.pdf") {
                            found_pdf_event = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }

        assert!(found_pdf_event, "should have detected the new PDF file");

        // Clean up — dropping dir will trigger watcher error and thread will exit.
        drop(dir);
        let _ = handle.join();
    }
}
