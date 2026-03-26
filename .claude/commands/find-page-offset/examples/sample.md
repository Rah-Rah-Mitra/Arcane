# Page Offset Report: pattern-recognition.pdf

**File:** /home/user/books/pattern-recognition.pdf
**Detection method:** PageLabels
**Confidence:** 95%

## Result

| Field | Value |
|-------|-------|
| Offset | +14 |
| Printed page 1 at physical page | 15 |
| Front matter pages | 14 (Roman numerals i–xiv) |

## Evidence

| Physical page | Printed label | Note |
|---------------|--------------|------|
| 1 | i | Roman front matter begins |
| 14 | xiv | Roman front matter ends |
| 15 | 1 | Arabic content begins |
| 16 | 2 | Consistent |
| 17 | 3 | Consistent |

## How to use this offset

```bash
# In recover-outline:
arcane recover-outline pattern-recognition.pdf --page-one 15 ...

# When adding to a project:
arcane add "PRML" pattern-recognition.pdf --textbook --start-page 14

# In find-offset (manual override):
arcane analyze offset pattern-recognition.pdf --toc-pages "5-12"
```

## Confidence interpretation

95% confidence from PageLabels — this is the most reliable method.
The /PageLabels dictionary directly encodes the front-matter/content boundary.
Use this value directly without further validation.
