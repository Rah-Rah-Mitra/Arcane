//! Command handler implementations — re-exported from focused sub-modules.
//!
//! # Module layout
//!
//! | Module        | Contents                                                  |
//! |---------------|-----------------------------------------------------------|
//! | `helpers`     | Shared parsers: page ranges, anchor pairs, temp paths     |
//! | `project`     | Project/source CRUD: new, add, list, show, remove, tag   |
//! | `pdf_ops`     | Base PDF operations: merge, split, rotate, protect, unlock, inject-outlines, extract-pages |
//! | `analyze`     | Analysis / inspection: probe, outline, layout, offset, sync-pages |
//! | `recover`     | Outline recovery pipeline: recover-outline, recover, recover-project, process-toc |
//! | `workflow`    | Project workflows: chunk, search, reindex, freq, tui, watch |

pub mod analyze;
pub mod helpers;
pub mod pdf_ops;
pub mod project;
pub mod recover;
pub mod workflow;

pub use analyze::*;
pub use helpers::*;
pub use pdf_ops::*;
pub use project::*;
pub use recover::*;
pub use workflow::*;
