//! Frequency dictionary generation from the Tantivy index.
//!
//! Iterates over every term in the `body` field for documents belonging
//! to a specific project, producing a `word → count` map sorted by
//! descending frequency.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::DocSet;

use super::indexer::{SearchIndex, FIELD_BODY, FIELD_PROJECT};

/// Entry in the frequency dictionary.
#[derive(Debug)]
pub struct FreqEntry {
    pub term: String,
    pub count: u64,
}

/// Build a frequency dictionary from documents belonging to `project_name`.
///
/// For each segment, finds the set of doc IDs matching the project, then
/// walks the `body` inverted index and sums term frequencies only for
/// those documents.
pub fn build_frequency_dict(index: &SearchIndex, project_name: &str) -> Result<Vec<FreqEntry>> {
    let reader = index
        .index()
        .reader()
        .context("failed to open index reader")?;

    let searcher = reader.searcher();
    let body_field = index.schema().get_field(FIELD_BODY).unwrap();
    let project_field = index.schema().get_field(FIELD_PROJECT).unwrap();
    let project_term = tantivy::Term::from_field_text(project_field, project_name);

    let mut freq_map: HashMap<String, u64> = HashMap::new();

    for segment_reader in searcher.segment_readers() {
        // Build the set of doc IDs that belong to this project.
        let project_index = segment_reader.inverted_index(project_field)?;
        let mut project_docs = HashSet::new();
        if let Some(mut postings) = project_index.read_postings(
            &project_term,
            tantivy::schema::IndexRecordOption::Basic,
        )? {
            while postings.advance() != tantivy::TERMINATED {
                project_docs.insert(postings.doc());
            }
        }

        if project_docs.is_empty() {
            continue;
        }

        // Walk every term in the body field, summing frequencies only for
        // documents in the project set.
        let body_index = segment_reader.inverted_index(body_field)?;
        let mut term_stream = body_index.terms().stream()?;

        while term_stream.advance() {
            let term_bytes = term_stream.key();
            if let Ok(term_str) = std::str::from_utf8(term_bytes) {
                let term = tantivy::Term::from_field_text(body_field, term_str);
                if let Some(mut postings) = body_index
                    .read_postings(&term, tantivy::schema::IndexRecordOption::WithFreqs)?
                {
                    use tantivy::postings::Postings as _;
                    let mut total = 0u64;
                    while postings.advance() != tantivy::TERMINATED {
                        if project_docs.contains(&postings.doc()) {
                            total += postings.term_freq() as u64;
                        }
                    }
                    if total > 0 {
                        *freq_map.entry(term_str.to_string()).or_insert(0) += total;
                    }
                }
            }
        }
    }

    let mut entries: Vec<FreqEntry> = freq_map
        .into_iter()
        .map(|(term, count)| FreqEntry { term, count })
        .collect();

    // Sort by frequency descending, then alphabetically for ties.
    entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term)));

    Ok(entries)
}

/// Write the frequency dictionary to a text file.
///
/// Format: `<word> <count>\n` — one entry per line, sorted by descending
/// frequency, matching the format shown in the user's example.
pub fn write_freq_file(entries: &[FreqEntry], output: &Path) -> Result<()> {
    let mut file = std::fs::File::create(output)
        .with_context(|| format!("failed to create {}", output.display()))?;

    for entry in entries {
        writeln!(file, "{} {}", entry.term, entry.count)
            .with_context(|| "failed to write frequency entry")?;
    }

    Ok(())
}
