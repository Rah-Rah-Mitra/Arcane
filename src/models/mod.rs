//! Domain models for Arcane.
//!
//! Pure data structures and domain logic. No I/O.

mod chunk;
mod project;
mod source;
mod tags;

pub use chunk::ChunkRecord;
pub use project::Project;
pub use source::{SourceMeta, build_source, Source, Textbook, Report};
pub use tags::SourceKind;
