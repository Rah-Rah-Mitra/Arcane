//! Filesystem layout helpers for Arcane.
//!
//! ```text
//! ~/Arcane/
//!   arcane.db
//!   Library/
//!     [Project_Name]/
//!       Originals/    ← symlinks or copies of the original PDFs
//!       Chunks/       ← split chapter PDFs
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Root paths
// ---------------------------------------------------------------------------

/// Returns `~/Arcane` (creates it if it does not yet exist).
pub fn arcane_root() -> Result<PathBuf> {
    let home = dirs_home()?;
    let root = home.join("Arcane");
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create Arcane root at {}", root.display()))?;
    Ok(root)
}

/// Returns `~/Arcane/Library/[project_name]`.
pub fn project_dir(project_name: &str) -> Result<PathBuf> {
    let dir = arcane_root()?.join("Library").join(project_name);
    Ok(dir)
}

/// Returns `~/Arcane/Library/[project_name]/Originals` (created on demand).
pub fn originals_dir(project_name: &str) -> Result<PathBuf> {
    let dir = project_dir(project_name)?.join("Originals");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create Originals directory for '{project_name}'"))?;
    Ok(dir)
}

/// Returns `~/Arcane/Library/[project_name]/Chunks` (created on demand).
pub fn chunks_dir(project_name: &str) -> Result<PathBuf> {
    let dir = project_dir(project_name)?.join("Chunks");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create Chunks directory for '{project_name}'"))?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Portable home-directory helper (no extra dependency)
// ---------------------------------------------------------------------------

pub(crate) fn dirs_home() -> Result<PathBuf> {
    // Prefer the HOME environment variable (works on Linux / macOS / WSL).
    if let Some(h) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(h));
    }
    // Windows fallback.
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(h));
    }
    anyhow::bail!("cannot determine home directory — neither HOME nor USERPROFILE is set")
}

// ---------------------------------------------------------------------------
// Symlink helper
// ---------------------------------------------------------------------------

/// Create a symlink in `originals_dir` pointing at the original PDF.
/// On platforms where symlinks are unavailable the file is copied instead.
pub fn link_original(project_name: &str, source_path: &Path) -> Result<PathBuf> {
    let target_dir = originals_dir(project_name)?;
    let file_name = source_path
        .file_name()
        .context("source path has no file name")?;
    let link_path = target_dir.join(file_name);

    if link_path.exists() {
        return Ok(link_path);
    }

    create_link_or_copy(source_path, &link_path)?;
    Ok(link_path)
}

/// Create a symlink in `originals_dir` pointing at a CAS blob, using the
/// original filename for the link name.
pub fn link_original_to_cas(
    project_name: &str,
    cas_blob_path: &Path,
    original_path: &Path,
) -> Result<PathBuf> {
    let target_dir = originals_dir(project_name)?;
    let file_name = original_path
        .file_name()
        .context("original path has no file name")?;
    let link_path = target_dir.join(file_name);

    if link_path.exists() {
        return Ok(link_path);
    }

    create_link_or_copy(cas_blob_path, &link_path)?;
    Ok(link_path)
}

/// Platform-aware helper: symlink on Unix, copy on Windows.
fn create_link_or_copy(source: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, link).with_context(|| {
            format!(
                "failed to create symlink {} → {}",
                link.display(),
                source.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(source, link).with_context(|| {
            format!(
                "failed to copy {} → {}",
                source.display(),
                link.display()
            )
        })?;
    }
    Ok(())
}
