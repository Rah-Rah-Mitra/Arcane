//! Database schema migrations for Arcane.
//!
//! Each migration is a named SQL script. On startup the database checks
//! which migrations have been applied and runs any pending ones in order.

use rusqlite::Connection;

use crate::error::StorageError;

/// Ordered list of migrations. Each entry is `(version_tag, sql)`.
const MIGRATIONS: &[(&str, &str)] = &[
    ("v1", include_str!("sql/v1_core.sql")),
    ("v2", include_str!("sql/v2_add_depth_pagecount.sql")),
];

/// Run all pending migrations on the given database connection.
pub fn run_pending(conn: &Connection) -> Result<(), StorageError> {
    // Ensure the schema_version table exists (bootstrap).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(StorageError::Database)?;

    for (version, sql) in MIGRATIONS {
        let already_applied: bool = conn
            .prepare("SELECT COUNT(*) FROM schema_version WHERE version = ?1")
            .map_err(StorageError::Database)?
            .query_row([version], |row| row.get::<_, i64>(0))
            .map(|count| count > 0)
            .map_err(StorageError::Database)?;

        if already_applied {
            continue;
        }

        tracing::info!("Applying migration {version}…");

        conn.execute_batch(sql)
            .map_err(|e| StorageError::MigrationFailed {
                version: version.to_string(),
                reason: e.to_string(),
            })?;

        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .map_err(StorageError::Database)?;
    }

    Ok(())
}
