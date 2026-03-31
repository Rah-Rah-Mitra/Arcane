# Skill: search

Search the full-text index for content across all indexed sources.

## When to use

- User asks to "search", "find", "look up" text in their documents
- User wants to know what books cover a topic
- User wants word frequency counts across a project (`freq`)
- Search returns stale results → run `reindex` first

## Steps Claude must follow

1. **Search**:
   ```bash
   # All projects:
   arcane search "<query>"

   # Scoped to a project:
   arcane search "<query>" --project "<project>"

   # Scoped to one source:
   arcane search "<query>" --project "<project>" --source "<title>"
   ```
   Show the user: result count, top hits (title, chapter, page).

2. **If results look stale or empty**:
   ```bash
   arcane reindex
   ```
   Then re-run the search. Reindex is safe to run at any time (idempotent).

3. **Word frequency** (when user wants "most common words", "term frequency", or to build a vocab list):
   ```bash
   arcane freq "<query>" --project "<project>"
   ```
   Output is `word<TAB>count` pairs, sorted descending.

4. Fill in `template.md` and present the summary.

## Notes

- `arcane reindex` reindexes **all** sources; there is no per-source flag.
- Sources must be added via `arcane add` before they can be indexed.
- Encrypted PDFs cannot be indexed until unlocked.
