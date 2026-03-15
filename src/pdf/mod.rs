//! PDF processing engine for Arcane.
//!
//! Contains the chunking logic, outline/bookmark extraction, PageLabel
//! resolution, structural operations, text extraction, and helpers.

pub mod clustering;
pub mod engine;
pub mod heuristics;
pub mod layout;
pub mod offset;
pub mod ops;
pub mod outlines;
pub mod page_labels;
pub mod pipeline;
pub mod probe;
pub mod seed;
pub mod text;
pub mod writer;

#[cfg(feature = "ocr")]
pub mod ocr;

// Re-export string helpers used across the PDF module.
pub(crate) use engine::{pdf_string_to_string, pdf_string_to_string_opt};
