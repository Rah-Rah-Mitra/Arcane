# Source Added: Computer-Vision / Multiple View Geometry

**File:** /home/user/books/multiple-view-geometry.pdf
**Type:** Textbook
**Document kind:** TextBased
**Total pages:** 672
**CAS hash:** a3f8c2d1…
**Deduplicated:** no

## Configuration

| Setting | Value |
|---------|-------|
| Needs chunking | yes |
| Start page (physical) | 22 |
| TOC page range | 7–18 |
| Tags | computer-vision, geometry |

## Index status

Reindexed 1 source, 672 pages total.
Search verified: `arcane search "epipolar geometry" --project Computer-Vision` → 4 results found.

## Recommended next steps

The PDF has no bookmarks (`has_outlines: false`). To split into chapters, first recover the outline:

```bash
# Option A — use TOC pages (Arcane-PP required):
arcane recover-outline-bridge

# Option B — heuristic recovery:
arcane recover-outline-heuristic
```

Then chunk with:
```bash
arcane chunk "Computer-Vision" --source "Multiple View Geometry" --depth 1
```
