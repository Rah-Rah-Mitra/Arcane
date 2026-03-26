# Skill: encrypt-decrypt-pdf

Password-protect a PDF (protect) or remove an existing password (unlock).
Both operations are lossless.

## When to use

- User says "encrypt", "password protect", "lock" → protect
- User says "decrypt", "unlock", "remove password" → unlock

## Protect steps

1. **Confirm the PDF is not already encrypted**:
   ```bash
   arcane analyze probe "<pdf>" --json
   ```

2. **Encrypt**:
   ```bash
   # Overwrite original:
   arcane pdf protect "<pdf>" --password "<password>"

   # Or write to new file (safer — preserves original):
   arcane pdf protect "<pdf>" --password "<password>" --output "<protected.pdf>"
   ```

3. **Verify**:
   ```bash
   arcane analyze probe "<protected.pdf>"
   ```

## Unlock steps

1. **Decrypt**:
   ```bash
   # Overwrite original:
   arcane pdf unlock "<pdf>" --password "<password>"

   # Or write to new file:
   arcane pdf unlock "<pdf>" --password "<password>" --output "<unlocked.pdf>"
   ```

2. **Verify**:
   ```bash
   arcane analyze probe "<unlocked.pdf>"
   ```

## Important notes

- Encrypted PDFs **cannot** be chunked, have outlines injected, or be analyzed until unlocked.
- Run `arcane pdf unlock` before any analysis or recovery workflow.
- The encryption uses PDF V1 RC4-40 — compatible with all PDF viewers.
- Claude must NOT log or echo passwords back to the user in plain text.

## Security reminder

Never store passwords in scripts or commit them to version control.
Use environment variables or pass them interactively if automating.
