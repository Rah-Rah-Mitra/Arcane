//! PDF processing engine for Arcane.
//!
//! Contains the chunking logic, outline/bookmark extraction, PageLabel
//! resolution, structural operations, text extraction, and helpers.

pub mod engine;
pub mod ops;
pub mod outlines;
pub mod page_labels;
pub mod text;
pub mod writer;

// Re-export string helpers used across the PDF module.
pub(crate) use engine::{pdf_string_to_string, pdf_string_to_string_opt};
