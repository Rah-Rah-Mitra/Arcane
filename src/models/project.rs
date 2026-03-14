//! The [`Project`] struct groups sources under a human-readable name.

use serde::{Deserialize, Serialize};

use super::SourceMeta;

/// A research project groups one or more [`SourceMeta`] entries under a
/// human-readable name and a set of optional tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique human-readable name (used as the directory name under `~/Arcane/Library/`).
    pub name: String,

    /// Optional keywords that help with organisation and search.
    #[serde(default)]
    pub tags: Vec<String>,

    /// All sources that belong to this project.
    #[serde(default)]
    pub sources: Vec<SourceMeta>,
}

impl Project {
    /// Create a new, empty project.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tags: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Add a source to the project.
    pub fn add_source(&mut self, meta: SourceMeta) {
        self.sources.push(meta);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn project_add_source() {
        let mut p = Project::new("Algorithms");
        p.tags.push("cs".into());
        assert_eq!(p.sources.len(), 0);

        let meta = SourceMeta::report("CLRS", PathBuf::from("/tmp/clrs.pdf"));
        p.add_source(meta);
        assert_eq!(p.sources.len(), 1);
        assert!(!p.sources[0].needs_chunking);
    }
}
