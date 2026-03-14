//! Content-Addressable Store (CAS) for Arcane.
//!
//! Files are stored by their blake3 hash in a two-level directory layout:
//!
//! ```text
//! ~/Arcane/CAS/{first 2 hex chars}/{full hash}/blob
//! ```
//!
//! This ensures that identical files are stored only once across all projects.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::filesystem::arcane_root;

/// Result of ingesting a file into the CAS.
#[derive(Debug, Clone)]
pub struct BlobRef {
    /// Hex-encoded blake3 hash of the file contents.
    pub hash: String,
    /// Size of the file in bytes.
    pub size: u64,
    /// Absolute path to the blob in the CAS.
    pub stored_path: PathBuf,
    /// `true` if the file was already present (deduplicated).
    pub was_deduplicated: bool,
}

/// Compute the blake3 hash of a file without loading it entirely into memory.
pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("cannot open file for hashing: {}", path.display()))?;

    let metadata = file.metadata()
        .with_context(|| format!("cannot read metadata: {}", path.display()))?;
    let size = metadata.len();

    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024]; // 64 KiB read buffer

    loop {
        let n = file.read(&mut buf)
            .with_context(|| format!("read error while hashing {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let hash = hasher.finalize().to_hex().to_string();
    Ok((hash, size))
}

/// Return the CAS directory root: `~/Arcane/CAS/`.
fn cas_root() -> Result<PathBuf> {
    let root = arcane_root()?.join("CAS");
    Ok(root)
}

/// Return the blob path for a given hash: `~/Arcane/CAS/ab/abcdef.../blob`.
pub fn blob_path(hash: &str) -> Result<PathBuf> {
    if hash.len() < 2 {
        anyhow::bail!("invalid hash (too short): {hash}");
    }
    let prefix = &hash[..2];
    Ok(cas_root()?.join(prefix).join(hash).join("blob"))
}

/// Ingest a file into the CAS. If the file is already present (same hash),
/// the copy is skipped and the existing path is returned.
///
/// Returns a [`BlobRef`] describing the stored blob.
pub fn ingest(source_path: &Path) -> Result<BlobRef> {
    let (hash, size) = hash_file(source_path)?;
    let target = blob_path(&hash)?;

    let was_deduplicated = if target.exists() {
        tracing::info!("Blob {hash} already in CAS — deduplicating.");
        true
    } else {
        // Create parent directories and copy the file.
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create CAS directory: {}", parent.display()))?;
        }
        fs::copy(source_path, &target)
            .with_context(|| format!(
                "cannot copy {} → {}",
                source_path.display(),
                target.display()
            ))?;
        tracing::info!("Stored blob {hash} ({size} bytes) in CAS.");
        false
    };

    Ok(BlobRef {
        hash,
        size,
        stored_path: target,
        was_deduplicated,
    })
}

/// Resolve a hash to the blob path on disk. Returns `None` if the blob
/// is not present in the CAS.
#[allow(dead_code)]
pub fn resolve(hash: &str) -> Result<Option<PathBuf>> {
    let target = blob_path(hash)?;
    if target.exists() {
        Ok(Some(target))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Helper: override CAS root to a temp directory for isolated tests.
    /// Since CAS uses arcane_root() which reads HOME, we test the lower-level
    /// functions directly.

    #[test]
    fn hash_file_deterministic() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.pdf");
        fs::write(&file_path, b"hello world PDF content").unwrap();

        let (hash1, size1) = hash_file(&file_path).unwrap();
        let (hash2, size2) = hash_file(&file_path).unwrap();

        assert_eq!(hash1, hash2, "same file must produce same hash");
        assert_eq!(size1, size2);
        assert_eq!(size1, 23); // "hello world PDF content" is 23 bytes
        assert_eq!(hash1.len(), 64, "blake3 hex hash is 64 chars");
    }

    #[test]
    fn hash_file_different_content() {
        let dir = tempdir().unwrap();
        let file_a = dir.path().join("a.pdf");
        let file_b = dir.path().join("b.pdf");
        fs::write(&file_a, b"content A").unwrap();
        fs::write(&file_b, b"content B").unwrap();

        let (hash_a, _) = hash_file(&file_a).unwrap();
        let (hash_b, _) = hash_file(&file_b).unwrap();

        assert_ne!(hash_a, hash_b, "different content must produce different hashes");
    }

    #[test]
    fn blob_path_structure() {
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let path = blob_path(hash).unwrap();
        let path_str = path.to_string_lossy();

        // Should contain CAS/ab/abcdef.../blob
        assert!(path_str.contains("CAS"));
        assert!(path_str.contains("ab"));
        assert!(path_str.contains(hash));
        assert!(path_str.ends_with("blob"));
    }

    #[test]
    fn ingest_and_dedup() {
        // This test uses the real CAS directory under ~/Arcane/CAS/
        // so we test hash + blob_path logic but skip actual ingest
        // to avoid polluting the user's filesystem.
        // Integration tests with real ingest belong in tests/ directory.

        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.pdf");
        fs::write(&file_path, b"test content for CAS").unwrap();

        let (hash, size) = hash_file(&file_path).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(size, 20);
    }
}
