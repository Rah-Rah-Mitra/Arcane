//! Source classification and tagging types.

use serde::{Deserialize, Serialize};

/// The kind of source document. Replaces the old boolean-only `needs_chunking`
/// dispatch with a richer taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceKind {
    Textbook,
    Report,
    Paper,
    Cheatsheet,
    Custom(String),
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Textbook => write!(f, "Textbook"),
            Self::Report => write!(f, "Report"),
            Self::Paper => write!(f, "Paper"),
            Self::Cheatsheet => write!(f, "Cheatsheet"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

impl Default for SourceKind {
    fn default() -> Self {
        Self::Report
    }
}
