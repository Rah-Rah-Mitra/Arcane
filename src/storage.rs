//! Storage layer for Arcane.
//!
//! Projects are persisted as `~/Arcane/projects.json`.  The module exposes a
//! simple [`ProjectStore`] that loads, queries, and saves the list of projects.
//!
//! File-system layout managed here:
//! ```
//! ~/Arcane/
//!   projects.json
//!   Library/
//!     [Project_Name]/
//!       Originals/    ← symlinks or copies of the original PDFs
//!       Chunks/       ← split chapter PDFs
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::Project;

// ---------------------------------------------------------------------------
// Root paths
// ---------------------------------------------------------------------------

/// Returns `~/Arcane` (creates it if it does not yet exist).
pub fn arcane_root() -> Result<PathBuf> {
    let home = dirs_home()?;
    let root = home.join("Arcane");
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create Arcane root at {}", root.display()))?;
    Ok(root)
}

/// Returns `~/Arcane/Library/[project_name]`.
pub fn project_dir(project_name: &str) -> Result<PathBuf> {
    let dir = arcane_root()?.join("Library").join(project_name);
    Ok(dir)
}

/// Returns `~/Arcane/Library/[project_name]/Originals` (created on demand).
pub fn originals_dir(project_name: &str) -> Result<PathBuf> {
    let dir = project_dir(project_name)?.join("Originals");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create Originals directory for '{project_name}'"))?;
    Ok(dir)
}

/// Returns `~/Arcane/Library/[project_name]/Chunks` (created on demand).
pub fn chunks_dir(project_name: &str) -> Result<PathBuf> {
    let dir = project_dir(project_name)?.join("Chunks");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create Chunks directory for '{project_name}'"))?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Portable home-directory helper (no extra dependency)
// ---------------------------------------------------------------------------

fn dirs_home() -> Result<PathBuf> {
    // Prefer the HOME environment variable (works on Linux / macOS / WSL).
    if let Some(h) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(h));
    }
    // Windows fallback.
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(h));
    }
    anyhow::bail!("cannot determine home directory — neither HOME nor USERPROFILE is set")
}

// ---------------------------------------------------------------------------
// ProjectStore
// ---------------------------------------------------------------------------

/// The on-disk envelope that wraps the list of all projects.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    projects: Vec<Project>,
}

/// A loaded view of `projects.json` that can be mutated and flushed.
pub struct ProjectStore {
    path: PathBuf,
    data: StoreData,
}

impl ProjectStore {
    /// Load (or create) the store from `~/Arcane/projects.json`.
    pub fn load() -> Result<Self> {
        let path = arcane_root()?.join("projects.json");
        Self::load_from(&path)
    }

    /// Load (or create) the store from an explicit path.  Useful in tests.
    pub fn load_from(path: &Path) -> Result<Self> {
        let data: StoreData = if path.exists() {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", path.display()))?
        } else {
            StoreData::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
        })
    }

    /// Persist all changes back to disk.
    pub fn save(&self) -> Result<()> {
        // Ensure the parent directory exists.
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.data)
            .context("failed to serialise project store")?;
        fs::write(&self.path, json)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        Ok(())
    }

    /// Return a slice of all projects.
    pub fn projects(&self) -> &[Project] {
        &self.data.projects
    }

    /// Return a mutable reference to a project by name, if it exists.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Project> {
        self.data.projects.iter_mut().find(|p| p.name == name)
    }

    /// Return an immutable reference to a project by name, if it exists.
    pub fn get(&self, name: &str) -> Option<&Project> {
        self.data.projects.iter().find(|p| p.name == name)
    }

    /// Insert or replace a project (matched by name).
    pub fn upsert(&mut self, project: Project) {
        if let Some(existing) = self.data.projects.iter_mut().find(|p| p.name == project.name) {
            *existing = project;
        } else {
            self.data.projects.push(project);
        }
    }

    /// Remove a project by name.  Returns `true` if a project was removed.
    #[allow(dead_code)]
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.data.projects.len();
        self.data.projects.retain(|p| p.name != name);
        self.data.projects.len() < before
    }
}

// ---------------------------------------------------------------------------
// Symlink helper
// ---------------------------------------------------------------------------

/// Create a symlink in `originals_dir` pointing at the original PDF.
/// On platforms where symlinks are unavailable the file is copied instead.
pub fn link_original(project_name: &str, source_path: &Path) -> Result<PathBuf> {
    let target_dir = originals_dir(project_name)?;
    let file_name = source_path
        .file_name()
        .context("source path has no file name")?;
    let link_path = target_dir.join(file_name);

    if link_path.exists() {
        return Ok(link_path);
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source_path, &link_path).with_context(|| {
            format!(
                "failed to create symlink {} → {}",
                link_path.display(),
                source_path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(source_path, &link_path).with_context(|| {
            format!(
                "failed to copy {} → {}",
                source_path.display(),
                link_path.display()
            )
        })?;
    }

    Ok(link_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Project;
    use tempfile::tempdir;

    #[test]
    fn round_trip_empty_store() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("projects.json");

        let store = ProjectStore::load_from(&path).unwrap();
        assert!(store.projects().is_empty());
        store.save().unwrap();

        // Reload
        let store2 = ProjectStore::load_from(&path).unwrap();
        assert!(store2.projects().is_empty());
    }

    #[test]
    fn upsert_and_remove() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("projects.json");

        let mut store = ProjectStore::load_from(&path).unwrap();

        store.upsert(Project::new("Algorithms"));
        store.upsert(Project::new("OS"));
        assert_eq!(store.projects().len(), 2);

        // Upsert same name updates in place
        let mut updated = Project::new("Algorithms");
        updated.tags.push("cs".into());
        store.upsert(updated);
        assert_eq!(store.projects().len(), 2);
        assert_eq!(store.get("Algorithms").unwrap().tags, vec!["cs"]);

        // Remove
        assert!(store.remove("OS"));
        assert_eq!(store.projects().len(), 1);
        assert!(!store.remove("OS")); // idempotent
    }

    #[test]
    fn persist_and_reload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("projects.json");

        let mut store = ProjectStore::load_from(&path).unwrap();
        let mut p = Project::new("Networks");
        p.tags.push("networking".into());
        store.upsert(p);
        store.save().unwrap();

        let store2 = ProjectStore::load_from(&path).unwrap();
        assert_eq!(store2.projects().len(), 1);
        assert_eq!(store2.projects()[0].name, "Networks");
        assert_eq!(store2.projects()[0].tags, vec!["networking"]);
    }
}
