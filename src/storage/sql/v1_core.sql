-- Arcane v1 core schema

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_tags (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (project_id, tag)
);

CREATE TABLE IF NOT EXISTS blobs (
    blake3_hash TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    stored_path TEXT NOT NULL,
    ingested_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    original_path TEXT NOT NULL,
    blob_hash TEXT REFERENCES blobs(blake3_hash),
    source_type TEXT NOT NULL DEFAULT 'Report',
    topic TEXT DEFAULT '',
    needs_chunking INTEGER NOT NULL DEFAULT 0,
    start_page_physical INTEGER,
    chapter_map_json TEXT DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_tags (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (source_id, tag)
);

CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    chapter_title TEXT NOT NULL,
    chapter_index INTEGER NOT NULL,
    start_page INTEGER NOT NULL,
    end_page INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    blob_hash TEXT REFERENCES blobs(blake3_hash),
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS search_meta (
    source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
    last_indexed_at TEXT,
    word_count INTEGER DEFAULT 0,
    page_count INTEGER DEFAULT 0,
    index_version INTEGER DEFAULT 0
);

-- Indices for common queries
CREATE INDEX IF NOT EXISTS idx_sources_project ON sources(project_id);
CREATE INDEX IF NOT EXISTS idx_sources_type ON sources(source_type);
CREATE INDEX IF NOT EXISTS idx_chunks_source ON chunks(source_id);
CREATE INDEX IF NOT EXISTS idx_project_tags_tag ON project_tags(tag);
CREATE INDEX IF NOT EXISTS idx_source_tags_tag ON source_tags(tag);
