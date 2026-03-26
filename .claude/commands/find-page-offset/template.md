# Page Offset Report: $PDF_NAME

**File:** $PDF_PATH
**Detection method:** $METHOD
**Confidence:** $CONFIDENCE%

## Result

| Field | Value |
|-------|-------|
| Offset | $OFFSET |
| Printed page 1 at physical page | $PAGE_ONE |
| Front matter pages | $FRONT_MATTER_COUNT |

## Evidence

$EVIDENCE_TABLE

## How to use this offset

```bash
# In recover-outline:
arcane recover-outline "$PDF_PATH" --page-one $PAGE_ONE ...

# When adding to a project:
arcane add "<project>" "$PDF_PATH" --textbook --start-page $START_PAGE_0BASED

# In find-offset (manual override):
arcane analyze offset "$PDF_PATH" --toc-pages "$TOC_PAGES"
```

## Confidence interpretation

$CONFIDENCE_EXPLANATION
