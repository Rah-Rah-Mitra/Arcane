//! Search and indexing layer for Arcane.
//!
//! Uses tantivy to provide full-text search across all projects and sources.
//! The index is stored at `~/Arcane/search_index/`.

pub mod indexer;
pub mod query;

pub use indexer::SearchIndex;
pub use query::{SearchResult, search};
