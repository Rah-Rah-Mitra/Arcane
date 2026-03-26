# Encryption Report: confidential-report.pdf

**Operation:** protect
**Input:** /home/user/docs/confidential-report.pdf
**Output:** /home/user/docs/confidential-report-protected.pdf

## Result

| Field | Value |
|-------|-------|
| Status | Success |
| Output file | /home/user/docs/confidential-report-protected.pdf |

## Verification

PDF parsed successfully after encryption. Document structure intact (TextBased, 42 pages).

## Notes

The original file was preserved. The protected file requires a password to open.
To remove the password later: `arcane pdf unlock confidential-report-protected.pdf --password <...> --output unlocked.pdf`
