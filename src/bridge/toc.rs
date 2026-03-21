use serde::{Deserialize, Serialize};

/// One table-of-contents entry returned by Arcane-PP `/parse-toc`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TocEntry {
    /// Full display title including any section numbering.
    pub title: String,
    /// 1-based logical page number parsed from TOC text.
    pub page: u32,
    /// Hierarchy depth (1 = chapter, 2 = section, ...).
    pub depth: u32,
}

#[cfg(test)]
mod tests {
    use super::TocEntry;

    #[test]
    fn serde_round_trip() {
        let entry = TocEntry {
            title: "1.2 Camera model".to_string(),
            page: 53,
            depth: 2,
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let parsed: TocEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, parsed);
    }
}
