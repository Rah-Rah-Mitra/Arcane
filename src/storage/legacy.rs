//! Legacy JSON storage — preserved for migration from v0.1 `projects.json`.
//!
//! This module contains the original `ProjectStore` that reads/writes
//! `~/Arcane/projects.json`.  It is kept for two purposes:
//!
//! 1. One-time migration from JSON to the new SQLite database.
//! 2. Backward-compatible usage during the transition period.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::Project;
use crate::storage::filesystem::arcane_root;

// ---------------------------------------------------------------------------
// StoreData
// ---------------------------------------------------------------------------

/// The on-disk envelope that wraps the list of all projects.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    projects: Vec<Project>,
}

// ---------------------------------------------------------------------------
// ProjectStore
// ---------------------------------------------------------------------------

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
        if let Some(existing) = self
            .data
            .projects
            .iter_mut()
            .find(|p| p.name == project.name)
        {
            *existing = project;
        } else {
            self.data.projects.push(project);
        }
    }

    /// Remove a project by name.  Returns `true` if a project was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.data.projects.len();
        self.data.projects.retain(|p| p.name != name);
        self.data.projects.len() < before
    }
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
