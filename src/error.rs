//! Typed errors used across the storage layer.
//!
//! Other modules rely on [`StorageError`] for database and filesystem errors.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("project '{name}' not found")]
    ProjectNotFound { name: String },

    #[error("source '{title}' not found in project '{project}'")]
    SourceNotFound { title: String, project: String },

    #[error("filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),

    #[error("migration failed at version {version}: {reason}")]
    MigrationFailed { version: String, reason: String },
}
