//! Domain models for Arcane.
//!
//! Pure data structures and domain logic. No I/O.

mod project;
mod source;

pub use project::Project;
pub use source::{build_source, SourceMeta};
