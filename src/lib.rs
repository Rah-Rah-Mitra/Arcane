//! Arcane — local-first research archival library.
//!
//! This crate provides the core logic for managing PDF research projects,
//! including chunking, search indexing, and storage.

pub mod cli;
pub mod error;
pub mod models;
pub mod pdf;
pub mod search;
pub mod storage;
pub mod ui;
pub mod watcher;
