//! Storage layer for Arcane.
//!
//! Provides filesystem layout management, database persistence via rusqlite,
//! and a legacy JSON migration path.

pub mod cas;
pub mod database;
pub mod filesystem;
pub mod legacy;
pub mod migrations;

pub use database::Database;
pub use filesystem::{arcane_root, project_dir, originals_dir, chunks_dir, link_original};
pub use legacy::ProjectStore;
