# Workflow: encrypt-decrypt-pdf

Password-protect a PDF or remove an existing password.
Both operations are lossless (no re-encoding).

## Base operations used

| Command | Operation |
|---------|-----------|
| `arcane pdf protect` | `pdf::ops::encrypt` (V1, RC4-40) |
| `arcane pdf unlock` | `pdf::ops::decrypt` |
| `arcane analyze probe` | Verify structure is intact after encryption |

## Encrypt (protect)

```bash
# Encrypt and overwrite the original
arcane pdf protect book.pdf --password "s3cr3t"

# Encrypt to a new file (preserves original)
arcane pdf protect book.pdf --password "s3cr3t" --output protected.pdf

# Verify the protected PDF still parses
arcane analyze probe protected.pdf
```

## Decrypt (unlock)

```bash
# Decrypt and overwrite
arcane pdf unlock protected.pdf --password "s3cr3t"

# Decrypt to a new file
arcane pdf unlock protected.pdf --password "s3cr3t" --output unlocked.pdf
```

## Notes

- Encryption uses PDF V1 (RC4 40-bit) — compatible with all PDF viewers
- The password applies to both user and owner passwords
- Encrypted PDFs cannot be chunked or have outlines injected until unlocked
- Run `arcane pdf unlock` before any analysis or recovery workflow
