//! Tantivy index management — schema definition, document indexing, and
//! index lifecycle.
//!
//! The index stores one document per PDF page with fields for source identity,
//! project membership, chapter context, and the full extracted text.

use std::path::Path;

use anyhow::{Context, Result};
use tantivy::schema::*;
use tantivy::{Index, IndexWriter, TantivyDocument};

use crate::pdf::text::PageText;
use crate::storage::filesystem::arcane_root;

/// Field names used in the tantivy schema.
pub const FIELD_SOURCE_ID: &str = "source_id";
pub const FIELD_PROJECT: &str = "project";
pub const FIELD_TITLE: &str = "title";
pub const FIELD_CHAPTER: &str = "chapter";
pub const FIELD_PAGE: &str = "page";
pub const FIELD_BODY: &str = "body";

/// Manages the tantivy full-text search index.
pub struct SearchIndex {
    index: Index,
    schema: Schema,
}

impl SearchIndex {
    /// Open or create the search index at `~/Arcane/search_index/`.
    pub fn open_or_create() -> Result<Self> {
        let index_dir = arcane_root()?.join("search_index");
        Self::open_at(&index_dir)
    }

    /// Open or create a search index at a specific directory.
    pub fn open_at(path: &Path) -> Result<Self> {
        let schema = Self::build_schema();

        let index = if path.exists() && path.join("meta.json").exists() {
            Index::open_in_dir(path)
                .with_context(|| format!("failed to open search index at {}", path.display()))?
        } else {
            std::fs::create_dir_all(path)
                .with_context(|| format!("failed to create index directory {}", path.display()))?;
            Index::create_in_dir(path, schema.clone())
                .with_context(|| "failed to create search index")?
        };

        Ok(Self { index, schema })
    }

    /// Create an in-memory index (for tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let schema = Self::build_schema();
        let index = Index::create_in_ram(schema.clone());
        Ok(Self { index, schema })
    }

    fn build_schema() -> Schema {
        let mut builder = Schema::builder();

        // Stored + indexed fields for filtering and display.
        builder.add_text_field(FIELD_SOURCE_ID, STRING | STORED);
        builder.add_text_field(FIELD_PROJECT, STRING | STORED);
        builder.add_text_field(FIELD_TITLE, TEXT | STORED);
        builder.add_text_field(FIELD_CHAPTER, TEXT | STORED);
        builder.add_u64_field(FIELD_PAGE, INDEXED | STORED);

        // Full-text body — indexed but not stored (too large).
        builder.add_text_field(FIELD_BODY, TEXT);

        builder.build()
    }

    /// Get a reference to the underlying tantivy index.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Get a reference to the schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Index a source's extracted pages.
    pub fn index_source(
        &self,
        source_id: &str,
        project_name: &str,
        source_title: &str,
        chapter_title: Option<&str>,
        pages: &[PageText],
    ) -> Result<u64> {
        let mut writer = self
            .index
            .writer(50_000_000) // 50 MB heap
            .context("failed to create index writer")?;

        let source_id_field = self.schema.get_field(FIELD_SOURCE_ID).unwrap();
        let project_field = self.schema.get_field(FIELD_PROJECT).unwrap();
        let title_field = self.schema.get_field(FIELD_TITLE).unwrap();
        let chapter_field = self.schema.get_field(FIELD_CHAPTER).unwrap();
        let page_field = self.schema.get_field(FIELD_PAGE).unwrap();
        let body_field = self.schema.get_field(FIELD_BODY).unwrap();

        let chapter = chapter_title.unwrap_or("");
        let mut indexed_count = 0u64;

        for page in pages {
            let text = page.text.trim();
            if text.is_empty() || page.word_count == 0 {
                continue;
            }

            let mut doc = TantivyDocument::new();
            doc.add_text(source_id_field, source_id);
            doc.add_text(project_field, project_name);
            doc.add_text(title_field, source_title);
            doc.add_text(chapter_field, chapter);
            doc.add_u64(page_field, page.page_index as u64);
            doc.add_text(body_field, text);

            writer.add_document(doc)?;
            indexed_count += 1;
        }

        writer.commit().context("failed to commit search index")?;

        tracing::info!(
            "Indexed {indexed_count} pages for '{source_title}' in project '{project_name}'."
        );

        Ok(indexed_count)
    }

    /// Remove all documents for a given source ID from the index.
    pub fn remove_source(&self, source_id: &str) -> Result<()> {
        let mut writer: IndexWriter<TantivyDocument> = self.index.writer(50_000_000)?;
        let source_id_field = self.schema.get_field(FIELD_SOURCE_ID).unwrap();

        let term = tantivy::Term::from_field_text(source_id_field, source_id);
        writer.delete_term(term);
        writer.commit()?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_in_memory_index() {
        let idx = SearchIndex::open_in_memory().unwrap();
        assert_eq!(idx.schema().num_fields(), 6);
    }

    #[test]
    fn index_and_count_pages() {
        let idx = SearchIndex::open_in_memory().unwrap();

        let pages = vec![
            PageText {
                page_index: 0,
                text: "Introduction to algorithms".into(),
                word_count: 3,
            },
            PageText {
                page_index: 1,
                text: "Sorting and searching techniques".into(),
                word_count: 4,
            },
            PageText {
                page_index: 2,
                text: "".into(),
                word_count: 0,
            }, // empty — should be skipped
        ];

        let count = idx
            .index_source("src-001", "Algorithms", "CLRS", Some("Chapter 1"), &pages)
            .unwrap();

        assert_eq!(count, 2, "empty pages should be skipped");
    }
}
