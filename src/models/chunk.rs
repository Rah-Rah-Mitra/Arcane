//! Chunk record — represents a single chapter extracted from a source PDF.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A record describing one chunk (chapter) extracted from a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ChunkRecord {
    /// Human-readable chapter title.
    pub chapter_title: String,

    /// 1-based ordering within the source.
    pub chapter_index: u32,

    /// 0-based physical start page (inclusive).
    pub start_page: u32,

    /// 0-based physical end page (inclusive).
    pub end_page: u32,

    /// Path to the chunk PDF file on disk.
    pub file_path: PathBuf,
}
