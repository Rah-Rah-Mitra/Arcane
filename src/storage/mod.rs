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
pub use filesystem::{chunks_dir, originals_dir};
pub use legacy::ProjectStore;
