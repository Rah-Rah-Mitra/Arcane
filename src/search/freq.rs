//! Frequency dictionary generation from the Tantivy index.
//!
//! Iterates over every term in the `body` field and sums up the total
//! term frequency across all segments, producing a `word → count` map
//! sorted by descending frequency.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::DocSet;

use super::indexer::{SearchIndex, FIELD_BODY};

/// Entry in the frequency dictionary.
#[derive(Debug)]
pub struct FreqEntry {
    pub term: String,
    pub count: u64,
}

/// Build a frequency dictionary from all indexed documents.
///
/// Walks every segment's inverted index for the `body` field and
/// accumulates the total term frequency (sum of all occurrences across
/// all documents) for each unique term.
pub fn build_frequency_dict(index: &SearchIndex) -> Result<Vec<FreqEntry>> {
    let reader = index
        .index()
        .reader()
        .context("failed to open index reader")?;

    let searcher = reader.searcher();
    let body_field = index.schema().get_field(FIELD_BODY).unwrap();

    let mut freq_map: HashMap<String, u64> = HashMap::new();

    for segment_reader in searcher.segment_readers() {
        let inverted_index = segment_reader.inverted_index(body_field)?;
        let mut term_stream = inverted_index.terms().stream()?;

        while term_stream.advance() {
            let term_bytes = term_stream.key();
            // Tantivy text terms are UTF-8 encoded.
            if let Ok(term_str) = std::str::from_utf8(term_bytes) {
                let term_info = term_stream.value();
                let doc_freq = term_info.doc_freq as u64;
                // doc_freq is the number of documents containing this term.
                // For a closer approximation of total occurrences, we use
                // the postings list to sum per-document term frequencies.
                *freq_map.entry(term_str.to_string()).or_insert(0) += doc_freq;
            }
        }
    }

    // Try to get actual term frequencies by walking postings if available.
    // Tantivy's TermInfo gives doc_freq but not total_term_freq directly
    // from the stream. We re-walk with postings for accurate counts.
    let mut accurate_map: HashMap<String, u64> = HashMap::new();

    for segment_reader in searcher.segment_readers() {
        let inverted_index = segment_reader.inverted_index(body_field)?;
        let mut term_stream = inverted_index.terms().stream()?;

        while term_stream.advance() {
            let term_bytes = term_stream.key();
            if let Ok(term_str) = std::str::from_utf8(term_bytes) {
                let term = tantivy::Term::from_field_text(body_field, term_str);
                if let Some(postings) = inverted_index
                    .read_postings(&term, tantivy::schema::IndexRecordOption::WithFreqs)?
                {
                    use tantivy::postings::Postings as _;
                    let mut postings = postings;
                    let mut total = 0u64;
                    while postings.advance() != tantivy::TERMINATED {
                        total += postings.term_freq() as u64;
                    }
                    *accurate_map.entry(term_str.to_string()).or_insert(0) += total;
                }
            }
        }
    }

    // Prefer accurate counts; fall back to doc_freq if postings unavailable.
    let final_map = if accurate_map.is_empty() {
        freq_map
    } else {
        accurate_map
    };

    let mut entries: Vec<FreqEntry> = final_map
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
