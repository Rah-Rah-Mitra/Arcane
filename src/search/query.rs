//! Query parsing and search result ranking.

use anyhow::{Context, Result};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value;
use tantivy::TantivyDocument;

use super::indexer::{SearchIndex, FIELD_BODY, FIELD_CHAPTER, FIELD_PAGE, FIELD_PROJECT, FIELD_SOURCE_ID, FIELD_TITLE};

/// A single search result with source context.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Source UUID.
    pub source_id: String,
    /// Project name.
    pub project_name: String,
    /// Source title.
    pub source_title: String,
    /// Chapter title (if available).
    pub chapter_title: String,
    /// Page number (0-based physical).
    pub page: u64,
    /// Relevance score.
    pub score: f32,
}

/// Search the index for documents matching the query string.
///
/// Returns up to `limit` results, sorted by relevance score (descending).
pub fn search(index: &SearchIndex, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let reader = index.index()
        .reader()
        .context("failed to open index reader")?;

    let searcher = reader.searcher();

    let body_field = index.schema().get_field(FIELD_BODY).unwrap();
    let title_field = index.schema().get_field(FIELD_TITLE).unwrap();
    let chapter_field = index.schema().get_field(FIELD_CHAPTER).unwrap();

    // Parse the query against both body and title fields.
    let query_parser = QueryParser::for_index(
        index.index(),
        vec![body_field, title_field, chapter_field],
    );

    let query = query_parser.parse_query(query_str)
        .with_context(|| format!("failed to parse search query: {query_str}"))?;

    let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))
        .context("search execution failed")?;

    let source_id_field = index.schema().get_field(FIELD_SOURCE_ID).unwrap();
    let project_field = index.schema().get_field(FIELD_PROJECT).unwrap();
    let page_field = index.schema().get_field(FIELD_PAGE).unwrap();

    let mut results = Vec::new();

    for (score, doc_address) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_address)
            .context("failed to retrieve document")?;

        let source_id = doc.get_first(source_id_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let project_name = doc.get_first(project_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let source_title = doc.get_first(title_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let chapter_title = doc.get_first(chapter_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let page = doc.get_first(page_field)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        results.push(SearchResult {
            source_id,
            project_name,
            source_title,
            chapter_title,
            page,
            score,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::text::PageText;

    #[test]
    fn search_finds_indexed_content() {
        let idx = SearchIndex::open_in_memory().unwrap();

        let pages = vec![
            PageText {
                page_index: 0,
                text: "Quantum mechanics describes the behavior of particles at atomic scales".into(),
                word_count: 10,
            },
            PageText {
                page_index: 1,
                text: "Classical mechanics applies to macroscopic objects like planets".into(),
                word_count: 8,
            },
        ];

        idx.index_source("src-qm", "Physics", "Griffiths", Some("Intro"), &pages).unwrap();

        // Search for "quantum"
        let results = search(&idx, "quantum", 10).unwrap();
        assert!(!results.is_empty(), "should find results for 'quantum'");
        assert_eq!(results[0].source_title, "Griffiths");
        assert_eq!(results[0].project_name, "Physics");

        // Search for something not in the index
        let results = search(&idx, "cryptocurrency", 10).unwrap();
        assert!(results.is_empty(), "should not find 'cryptocurrency'");
    }

    #[test]
    fn search_across_sources() {
        let idx = SearchIndex::open_in_memory().unwrap();

        let pages_a = vec![
            PageText { page_index: 0, text: "Graph algorithms and shortest paths".into(), word_count: 5 },
        ];
        let pages_b = vec![
            PageText { page_index: 0, text: "Network protocols and routing algorithms".into(), word_count: 5 },
        ];

        idx.index_source("src-a", "CS", "CLRS", None, &pages_a).unwrap();
        idx.index_source("src-b", "CS", "Tanenbaum", None, &pages_b).unwrap();

        let results = search(&idx, "algorithms", 10).unwrap();
        assert_eq!(results.len(), 2, "should find 'algorithms' in both sources");
    }
}
