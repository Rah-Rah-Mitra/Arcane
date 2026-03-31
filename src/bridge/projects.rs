use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct ContentsPageRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Deserialize)]
pub struct SourceMeta {
    pub title: String,
    pub path: String,
    pub chapter_map: serde_json::Value,
    pub contents_page_range: Option<ContentsPageRange>,
}

impl SourceMeta {
    pub fn chapter_map_is_empty(&self) -> bool {
        self.chapter_map.as_object().is_none_or(|m| m.is_empty())
    }

    pub fn pdf_path(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub name: String,
    pub sources: Vec<SourceMeta>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectStore {
    pub projects: Vec<Project>,
}

/// Load `projects.json` from an Arcane data root directory.
pub fn load_projects(arcane_data_dir: &Path) -> anyhow::Result<ProjectStore> {
    let path = arcane_data_dir.join("projects.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let store: ProjectStore = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(store)
}

/// Return sources in `project_name` with empty chapter_map and known TOC range.
pub fn sources_needing_recovery<'a>(
    store: &'a ProjectStore,
    project_name: &str,
) -> Vec<&'a SourceMeta> {
    store
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(project_name))
        .map(|proj| {
            proj.sources
                .iter()
                .filter(|s| s.chapter_map_is_empty() && s.contents_page_range.is_some())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{load_projects, sources_needing_recovery};

    #[test]
    fn filters_only_sources_needing_recovery() {
        let json = r#"
        {
          "projects": [
            {
              "name": "Computer-Vision",
              "sources": [
                {
                  "title": "NeedsRecovery",
                  "path": "C:/books/a.pdf",
                  "chapter_map": {},
                  "contents_page_range": {"start": 7, "end": 18}
                },
                {
                  "title": "AlreadyRecovered",
                  "path": "C:/books/b.pdf",
                  "chapter_map": {"0": "Intro"},
                  "contents_page_range": {"start": 7, "end": 18}
                },
                {
                  "title": "NoTocRange",
                  "path": "C:/books/c.pdf",
                  "chapter_map": {},
                  "contents_page_range": null
                }
              ]
            }
          ]
        }
        "#;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("projects.json");
        std::fs::write(&path, json).expect("write projects.json");

        let store = load_projects(dir.path()).expect("load projects");
        let items = sources_needing_recovery(&store, "computer-vision");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "NeedsRecovery");
    }
}
