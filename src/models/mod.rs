//! Domain models for Arcane.
//!
//! Pure data structures and domain logic. No I/O.

mod chunk;
mod project;
mod source;
mod tags;

pub use project::Project;
pub use source::{build_source, ContentsPageRange, SourceMeta};
