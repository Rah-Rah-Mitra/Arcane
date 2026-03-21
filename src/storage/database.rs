//! SQLite-backed storage for Arcane.
//!
//! Wraps a [`rusqlite::Connection`] and implements the core CRUD operations
//! for projects, sources, chunks, tags, and blobs.

use rusqlite::Connection;

use crate::error::StorageError;
use crate::models::Project;
use crate::storage::filesystem;
use crate::storage::migrations;

/// The primary database handle for Arcane.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the Arcane database at `~/Arcane/arcane.db` and run
    /// any pending migrations.
    pub fn open_or_create() -> Result<Self, StorageError> {
        let root = filesystem::arcane_root()
            .map_err(|e| StorageError::Filesystem(std::io::Error::other(e.to_string())))?;
        let db_path = root.join("arcane.db");
        Self::open_at(&db_path)
    }

    /// Open (or create) a database at a specific path. Useful for testing.
    pub fn open_at(path: &std::path::Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;

        // Enable WAL mode for better concurrent read performance.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        migrations::run_pending(&conn)?;

        Ok(Self { conn })
    }

    /// Open an in-memory database (for tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        migrations::run_pending(&conn)?;
        Ok(Self { conn })
    }

    /// Get a reference to the underlying connection.
    #[allow(dead_code)]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // ── Project operations ───────────────────────────────────────────────

    /// Create a new project. Returns the generated UUID.
    pub fn create_project(&self, name: &str) -> Result<String, StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, name, now, now],
        )?;

        Ok(id)
    }

    /// List all projects (without sources — use `get_project` for full detail).
    #[allow(dead_code)]
    pub fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM projects ORDER BY name")?;

        let projects = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                Ok(Project::new(name))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Populate tags for each project.
        let mut result = Vec::new();
        for mut p in projects {
            p.tags = self.get_project_tags(&p.name)?;
            result.push(p);
        }

        Ok(result)
    }

    /// Check if a project exists by name.
    pub fn project_exists(&self, name: &str) -> Result<bool, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM projects WHERE name = ?1",
            [name],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the project UUID by name.
    pub fn get_project_id(&self, name: &str) -> Result<Option<String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM projects WHERE name = ?1")?;
        let mut rows = stmt.query([name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Delete a project by name. Returns true if it existed.
    /// FK cascades handle sources, tags, chunks, and search_meta.
    pub fn delete_project(&self, name: &str) -> Result<bool, StorageError> {
        let affected = self
            .conn
            .execute("DELETE FROM projects WHERE name = ?1", [name])?;
        Ok(affected > 0)
    }

    /// Get all source titles for a project (used before deletion to clean up search index).
    #[allow(dead_code)]
    pub fn get_source_titles(&self, project_name: &str) -> Result<Vec<String>, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        let mut stmt = self
            .conn
            .prepare("SELECT title FROM sources WHERE project_id = ?1")?;
        let titles = stmt
            .query_map([&project_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(titles)
    }

    /// Delete a source by project name and title. Returns the blob_hash if one
    /// was associated (caller can use it for CAS cleanup).
    /// FK cascades handle source_tags, chunks, and search_meta.
    pub fn delete_source(
        &self,
        project_name: &str,
        source_title: &str,
    ) -> Result<Option<String>, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        // Fetch blob_hash before deleting.
        let blob_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT blob_hash FROM sources WHERE project_id = ?1 AND title = ?2",
                rusqlite::params![project_id, source_title],
                |row| row.get(0),
            )
            .ok();

        self.conn.execute(
            "DELETE FROM sources WHERE project_id = ?1 AND title = ?2",
            rusqlite::params![project_id, source_title],
        )?;

        Ok(blob_hash)
    }

    // ── Tag operations ───────────────────────────────────────────────────

    /// Get all tags for a project.
    #[allow(dead_code)]
    pub fn get_project_tags(&self, project_name: &str) -> Result<Vec<String>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT pt.tag FROM project_tags pt
             JOIN projects p ON pt.project_id = p.id
             WHERE p.name = ?1
             ORDER BY pt.tag",
        )?;
        let tags = stmt
            .query_map([project_name], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    /// Add a tag to a project.
    #[allow(dead_code)]
    pub fn add_project_tag(&self, project_name: &str, tag: &str) -> Result<(), StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        self.conn.execute(
            "INSERT OR IGNORE INTO project_tags (project_id, tag) VALUES (?1, ?2)",
            rusqlite::params![project_id, tag],
        )?;
        Ok(())
    }

    /// Remove a tag from a project.
    #[allow(dead_code)]
    pub fn remove_project_tag(&self, project_name: &str, tag: &str) -> Result<bool, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        let affected = self.conn.execute(
            "DELETE FROM project_tags WHERE project_id = ?1 AND tag = ?2",
            rusqlite::params![project_id, tag],
        )?;
        Ok(affected > 0)
    }

    /// Find all projects that have a given tag.
    #[allow(dead_code)]
    pub fn find_projects_by_tag(&self, tag: &str) -> Result<Vec<String>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT p.name FROM projects p
             JOIN project_tags pt ON pt.project_id = p.id
             WHERE pt.tag = ?1
             ORDER BY p.name",
        )?;
        let names = stmt
            .query_map([tag], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    /// Add a tag to a source (by source title within a project).
    #[allow(dead_code)]
    pub fn add_source_tag(
        &self,
        project_name: &str,
        source_title: &str,
        tag: &str,
    ) -> Result<(), StorageError> {
        let source_id = self
            .get_source_id(project_name, source_title)?
            .ok_or_else(|| StorageError::SourceNotFound {
                title: source_title.to_string(),
                project: project_name.to_string(),
            })?;

        self.conn.execute(
            "INSERT OR IGNORE INTO source_tags (source_id, tag) VALUES (?1, ?2)",
            rusqlite::params![source_id, tag],
        )?;
        Ok(())
    }

    /// Get all tags for a source.
    #[allow(dead_code)]
    pub fn get_source_tags(
        &self,
        project_name: &str,
        source_title: &str,
    ) -> Result<Vec<String>, StorageError> {
        let source_id = self
            .get_source_id(project_name, source_title)?
            .ok_or_else(|| StorageError::SourceNotFound {
                title: source_title.to_string(),
                project: project_name.to_string(),
            })?;

        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM source_tags WHERE source_id = ?1 ORDER BY tag")?;
        let tags = stmt
            .query_map([&source_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    /// Get a source UUID by project name and source title.
    fn get_source_id(
        &self,
        project_name: &str,
        source_title: &str,
    ) -> Result<Option<String>, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        let mut stmt = self
            .conn
            .prepare("SELECT id FROM sources WHERE project_id = ?1 AND title = ?2")?;
        let mut rows = stmt.query(rusqlite::params![project_id, source_title])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    // ── Blob / CAS operations ────────────────────────────────────────────

    /// Register a blob in the database. Idempotent — does nothing if the
    /// hash already exists.
    pub fn register_blob(
        &self,
        hash: &str,
        size: u64,
        stored_path: &str,
    ) -> Result<(), StorageError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO blobs (blake3_hash, size_bytes, stored_path, ingested_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![hash, size as i64, stored_path, now],
        )?;
        Ok(())
    }

    /// Check whether a blob with the given hash exists in the database.
    #[allow(dead_code)]
    pub fn blob_exists(&self, hash: &str) -> Result<bool, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM blobs WHERE blake3_hash = ?1",
            [hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the stored path of a blob by its hash.
    #[allow(dead_code)]
    pub fn get_blob_path(&self, hash: &str) -> Result<Option<String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT stored_path FROM blobs WHERE blake3_hash = ?1")?;
        let mut rows = stmt.query([hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    // ── Source operations ─────────────────────────────────────────────────

    /// Add a source to a project. Returns the generated source UUID.
    #[allow(clippy::too_many_arguments)]
    pub fn add_source(
        &self,
        project_name: &str,
        title: &str,
        original_path: &str,
        blob_hash: Option<&str>,
        source_type: &str,
        needs_chunking: bool,
        start_page_physical: Option<u32>,
        chapter_map_json: &str,
    ) -> Result<String, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO sources (id, project_id, title, original_path, blob_hash,
             source_type, needs_chunking, start_page_physical, chapter_map_json,
             depth, page_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                id,
                project_id,
                title,
                original_path,
                blob_hash,
                source_type,
                needs_chunking as i32,
                start_page_physical.map(|v| v as i64),
                chapter_map_json,
                None::<i64>, // depth - initially None
                None::<i64>, // page_count - initially None
                now,
                now
            ],
        )?;

        Ok(id)
    }

    /// Get all sources for a project, returned as `SourceMeta` for backward
    /// compatibility with the chunking pipeline.
    #[allow(dead_code)]
    pub fn get_sources(
        &self,
        project_name: &str,
    ) -> Result<Vec<crate::models::SourceMeta>, StorageError> {
        let project_id =
            self.get_project_id(project_name)?
                .ok_or_else(|| StorageError::ProjectNotFound {
                    name: project_name.to_string(),
                })?;

        let mut stmt = self.conn.prepare(
            "SELECT title, original_path, needs_chunking, start_page_physical, chapter_map_json,
             depth, page_count
             FROM sources WHERE project_id = ?1",
        )?;

        let sources = stmt
            .query_map([&project_id], |row| {
                let title: String = row.get(0)?;
                let path_str: String = row.get(1)?;
                let needs_chunking: i32 = row.get(2)?;
                let start_page: Option<i64> = row.get(3)?;
                let chapter_json: String = row.get(4)?;
                let depth: Option<i64> = row.get(5)?;
                let page_count: Option<i64> = row.get(6)?;

                Ok((
                    title,
                    path_str,
                    needs_chunking,
                    start_page,
                    chapter_json,
                    depth,
                    page_count,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (title, path_str, needs_chunking, start_page, chapter_json, depth, page_count) in
            sources
        {
            let chapter_map: std::collections::HashMap<u32, String> =
                serde_json::from_str(&chapter_json).unwrap_or_default();

            let meta = crate::models::SourceMeta {
                title,
                path: std::path::PathBuf::from(path_str),
                needs_chunking: needs_chunking != 0,
                chapter_map,
                start_page_physical: start_page.map(|v| v as u32),
                depth: depth.map(|v| v as u32),
                page_count: page_count.map(|v| v as u32),
                contents_page_range: None,
            };
            result.push(meta);
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list_projects() {
        let db = Database::open_in_memory().unwrap();

        db.create_project("Algorithms").unwrap();
        db.create_project("Networks").unwrap();

        let projects = db.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "Algorithms");
        assert_eq!(projects[1].name, "Networks");
    }

    #[test]
    fn project_exists_check() {
        let db = Database::open_in_memory().unwrap();

        assert!(!db.project_exists("Ghost").unwrap());
        db.create_project("Ghost").unwrap();
        assert!(db.project_exists("Ghost").unwrap());
    }

    #[test]
    fn delete_project() {
        let db = Database::open_in_memory().unwrap();

        db.create_project("Temp").unwrap();
        assert!(db.delete_project("Temp").unwrap());
        assert!(!db.delete_project("Temp").unwrap()); // idempotent
        assert!(!db.project_exists("Temp").unwrap());
    }

    #[test]
    fn project_tags() {
        let db = Database::open_in_memory().unwrap();

        db.create_project("Algorithms").unwrap();
        db.add_project_tag("Algorithms", "cs").unwrap();
        db.add_project_tag("Algorithms", "core").unwrap();
        db.add_project_tag("Algorithms", "cs").unwrap(); // duplicate — ignored

        let tags = db.get_project_tags("Algorithms").unwrap();
        assert_eq!(tags, vec!["core", "cs"]);
    }

    #[test]
    fn remove_project_tag() {
        let db = Database::open_in_memory().unwrap();
        db.create_project("TestProj").unwrap();

        db.add_project_tag("TestProj", "math").unwrap();
        db.add_project_tag("TestProj", "science").unwrap();
        assert_eq!(db.get_project_tags("TestProj").unwrap().len(), 2);

        assert!(db.remove_project_tag("TestProj", "math").unwrap());
        assert_eq!(db.get_project_tags("TestProj").unwrap(), vec!["science"]);

        // Removing non-existent tag returns false
        assert!(!db.remove_project_tag("TestProj", "math").unwrap());
    }

    #[test]
    fn find_projects_by_tag() {
        let db = Database::open_in_memory().unwrap();
        db.create_project("Alpha").unwrap();
        db.create_project("Beta").unwrap();
        db.create_project("Gamma").unwrap();

        db.add_project_tag("Alpha", "cs").unwrap();
        db.add_project_tag("Beta", "cs").unwrap();
        db.add_project_tag("Gamma", "math").unwrap();

        let cs_projects = db.find_projects_by_tag("cs").unwrap();
        assert_eq!(cs_projects, vec!["Alpha", "Beta"]);

        let math_projects = db.find_projects_by_tag("math").unwrap();
        assert_eq!(math_projects, vec!["Gamma"]);

        let empty = db.find_projects_by_tag("nonexistent").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn source_tags() {
        let db = Database::open_in_memory().unwrap();
        db.create_project("Physics").unwrap();

        db.add_source(
            "Physics",
            "QM Textbook",
            "/path/to/qm.pdf",
            None,
            "Textbook",
            true,
            None,
            "{}",
        )
        .unwrap();

        db.add_source_tag("Physics", "QM Textbook", "quantum")
            .unwrap();
        db.add_source_tag("Physics", "QM Textbook", "advanced")
            .unwrap();
        db.add_source_tag("Physics", "QM Textbook", "quantum")
            .unwrap(); // dup

        let tags = db.get_source_tags("Physics", "QM Textbook").unwrap();
        assert_eq!(tags, vec!["advanced", "quantum"]);
    }

    #[test]
    fn register_and_check_blob() {
        let db = Database::open_in_memory().unwrap();

        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        assert!(!db.blob_exists(hash).unwrap());

        db.register_blob(hash, 1024, "/cas/ab/abcdef.../blob")
            .unwrap();
        assert!(db.blob_exists(hash).unwrap());

        // Idempotent — second insert is ignored
        db.register_blob(hash, 1024, "/cas/ab/abcdef.../blob")
            .unwrap();
        assert!(db.blob_exists(hash).unwrap());

        let path = db.get_blob_path(hash).unwrap();
        assert_eq!(path, Some("/cas/ab/abcdef.../blob".to_string()));
    }

    #[test]
    fn add_and_get_sources() {
        let db = Database::open_in_memory().unwrap();
        db.create_project("Physics").unwrap();

        // Register the blob first to satisfy the FK constraint.
        let blob_hash = "deadbeef1234567890abcdef1234567890abcdef1234567890abcdef12345678";
        db.register_blob(blob_hash, 5000, "/cas/de/deadbeef.../blob")
            .unwrap();

        db.add_source(
            "Physics",
            "Griffiths QM",
            "/home/user/griffiths.pdf",
            Some(blob_hash),
            "Textbook",
            true,
            Some(14),
            r#"{"0":"Front Matter","14":"Chapter 1"}"#,
        )
        .unwrap();

        db.add_source(
            "Physics",
            "Formula Sheet",
            "/home/user/formulas.pdf",
            None,
            "Report",
            false,
            None,
            "{}",
        )
        .unwrap();

        let sources = db.get_sources("Physics").unwrap();
        assert_eq!(sources.len(), 2);

        assert_eq!(sources[0].title, "Griffiths QM");
        assert!(sources[0].needs_chunking);
        assert_eq!(sources[0].start_page_physical, Some(14));
        assert_eq!(sources[0].chapter_map.len(), 2);

        assert_eq!(sources[1].title, "Formula Sheet");
        assert!(!sources[1].needs_chunking);
    }
}
