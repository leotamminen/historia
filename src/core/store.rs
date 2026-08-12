//! Locate and open `.historia/` (walking up from the current directory, like git);
//! read and write content-addressed blobs under `objects/` by their hash; the
//! `.historia/lock` primitive (acquire/release, fail-fast, stale-lock reporting).
//! CLAUDE.md §8 (core/store.rs), §9 (on-disk layout), Rule 6 (locking).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::{fsutil, snapshot};
use crate::format::manifest;

/// Name of the store directory under a tracked folder (CLAUDE.md §9).
pub const STORE_DIR_NAME: &str = ".historia";

/// Why `init_store` could not create a new store.
#[derive(Debug)]
pub enum InitError {
    /// A store already exists at the target - `init_store` never overwrites it.
    AlreadyExists,
    /// Some other filesystem operation failed while building the store.
    Io(io::Error),
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::AlreadyExists => write!(f, "store already exists"),
            InitError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InitError {}

impl From<io::Error> for InitError {
    fn from(e: io::Error) -> Self {
        InitError::Io(e)
    }
}

/// Create a fresh `.historia/` store under `target_dir` (which must already exist):
/// `objects/`, `snapshots/`, `format`, and `HEAD` (CLAUDE.md §9). `lock` is
/// deliberately not created here - it exists only while a `commit`/`restore` is
/// running (CP2/CP3, Rule 6).
///
/// The whole store is built in a staging directory next to the target and then
/// moved into place with a single rename, so a failure partway through never
/// leaves a `.historia/` that reads as valid but is actually incomplete (CLAUDE.md
/// §6 requirement, Rule 5's spirit applied to the whole store, not just one file).
pub fn init_store(target_dir: &Path) -> Result<PathBuf, InitError> {
    let store_dir = target_dir.join(STORE_DIR_NAME);
    if store_dir.exists() {
        return Err(InitError::AlreadyExists);
    }

    let staging_dir = target_dir.join(format!(".{STORE_DIR_NAME}.init-{}.tmp", std::process::id()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }

    if let Err(e) = build_staging_store(&staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    // Re-check right before the rename: another process could have won an init
    // race while we were building the staging dir. Back off rather than clobber.
    if store_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(InitError::AlreadyExists);
    }

    if let Err(e) = fs::rename(&staging_dir, &store_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(InitError::Io(e));
    }

    Ok(store_dir)
}

fn build_staging_store(staging_dir: &Path) -> Result<(), InitError> {
    fs::create_dir_all(staging_dir.join("objects"))?;
    fs::create_dir_all(staging_dir.join("snapshots"))?;
    fsutil::write_atomic(&staging_dir.join("format"), manifest::FORMAT_MARKER.as_bytes())?;
    fsutil::write_atomic(&staging_dir.join("HEAD"), snapshot::INITIAL_HEAD.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn creates_the_full_store_layout() {
        let dir = tempdir().unwrap();

        let store_dir = init_store(dir.path()).unwrap();

        assert_eq!(store_dir, dir.path().join(STORE_DIR_NAME));
        assert!(store_dir.join("objects").is_dir());
        assert!(store_dir.join("snapshots").is_dir());
        assert!(!store_dir.join("lock").exists());
        assert_eq!(
            fs::read_to_string(store_dir.join("format")).unwrap(),
            crate::format::manifest::FORMAT_MARKER
        );
        assert_eq!(
            fs::read_to_string(store_dir.join("HEAD")).unwrap(),
            crate::core::snapshot::INITIAL_HEAD
        );
    }

    #[test]
    fn second_init_fails_and_leaves_first_store_untouched() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let head_before = fs::read_to_string(store_dir.join("HEAD")).unwrap();
        let format_before = fs::read_to_string(store_dir.join("format")).unwrap();

        let err = init_store(dir.path()).unwrap_err();

        assert!(matches!(err, InitError::AlreadyExists));
        assert_eq!(fs::read_to_string(store_dir.join("HEAD")).unwrap(), head_before);
        assert_eq!(fs::read_to_string(store_dir.join("format")).unwrap(), format_before);
    }

    #[test]
    fn leaves_no_staging_leftovers_after_success() {
        let dir = tempdir().unwrap();

        init_store(dir.path()).unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .filter(|n| n != STORE_DIR_NAME)
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    }
}
