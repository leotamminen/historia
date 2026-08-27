//! Atomic write-then-rename helpers, and the safe (mirror) restore routine that
//! makes the working folder match a snapshot exactly without ever touching ignored
//! paths (CLAUDE.md Rule 3, Rule 4, Rule 5).

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::core::store;
use crate::core::walk;
use crate::format::manifest::Entry;

/// Write to `path` atomically: run `write` against a sibling temp file, `fsync`
/// it, then rename over the destination - or, on failure, remove the temp file
/// and leave `path` untouched. A crash or power loss can never leave `path`
/// holding partial contents - readers see either the old contents or the new
/// ones, never a mix (CLAUDE.md Rule 5). Shared by [`write_atomic`] (whole
/// buffer) and [`restore_entry`] (streamed from a blob, Rule 11).
fn write_atomic_with(path: &Path, write: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "write_atomic: path has no parent directory")
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "write_atomic: path has no file name")
    })?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp_path)?;
        write(&mut file)?;
        file.sync_all()
    })();

    match result {
        Ok(()) => {
            fs::rename(&tmp_path, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

/// Write `contents` to `path` atomically (write-then-rename, Rule 5).
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| file.write_all(contents))
}

/// Render an absolute path for a user-facing message. `canonicalize()` prefixes
/// Windows paths with the verbatim `\\?\` marker; that form is correct but noisy
/// to read in a CLI message, so strip it for display only - the underlying path
/// used for actual filesystem operations is unaffected. Shared by every command
/// that prints an absolute path back to the user (`init`, CP11's `backup`, ...).
#[cfg(windows)]
pub fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

#[cfg(not(windows))]
pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// Restore statistics for a whole-folder [`mirror_restore`]: how many entries
/// were written, and how many tracked-but-not-in-the-target-snapshot files were
/// deleted (Rule 4).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MirrorStats {
    pub written: usize,
    pub deleted: usize,
}

/// Restore one manifest entry's content from the blob store to
/// `root.join(&entry.path)`: streamed straight from the blob to a sibling temp
/// file (Rule 11 - never buffered whole in memory) then renamed into place
/// (Rule 5), creating parent directories as needed. Applies the stored mode on a
/// best-effort basis (Rule 7) - restoring content is what matters; round-trip
/// equality is defined on content, not exact mode bits.
pub fn restore_entry(store_dir: &Path, root: &Path, entry: &Entry) -> io::Result<()> {
    let dest = root.join(&entry.path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut blob = store::open_blob(store_dir, &entry.hash)?;
    write_atomic_with(&dest, |file| {
        io::copy(&mut blob, file)?;
        Ok(())
    })?;

    apply_mode(&dest, entry.mode);
    Ok(())
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    // Best-effort (Rule 7): a failure here (e.g. a filesystem without POSIX
    // permission bits) must not fail the restore - content is what matters.
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

/// Windows has no POSIX exec bit (Rule 7) - best-effort here means "do nothing".
#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: u32) {}

/// Make the tracked set under `root` exactly match `entries` (Rule 4): write
/// every entry's content (via [`restore_entry`]), then delete every currently
/// tracked file not present in `entries`. "Tracked" means "would be walked"
/// under the same default ignores `commit`/`status` use (`core::walk`) - ignored
/// paths (`.git`, `node_modules`, `target`, `dist`, `build`, `.historia/`) are
/// never walked, so they are never touched here either. Does not attempt to
/// remove now-empty directories (CLAUDE.md CP6: not worth the risk).
pub fn mirror_restore(store_dir: &Path, root: &Path, entries: &[Entry]) -> io::Result<MirrorStats> {
    for entry in entries {
        restore_entry(store_dir, root, entry)?;
    }

    let target_paths: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.path.as_str()).collect();
    let walked = walk::walk(root)?;

    let mut deleted = 0;
    for file in &walked.files {
        if !target_paths.contains(file.relative_path.as_str()) {
            fs::remove_file(&file.absolute_path)?;
            deleted += 1;
        }
    }

    Ok(MirrorStats { written: entries.len(), deleted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn writes_new_file_with_given_contents() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.txt");

        write_atomic(&path, b"hello").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn overwrites_existing_file_and_leaves_no_tmp_file_behind() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.txt");

        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(entries.len(), 1, "expected only the target file, found {entries:?}");
    }

    // ---- restore_entry / mirror_restore (CP6) ----

    use crate::core::store;
    use crate::format::manifest::Entry;

    #[test]
    fn restore_entry_writes_blob_content_to_the_target_path_creating_parent_dirs() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        let hash = store::write_blob(&store_dir, &mut &b"hello restore"[..]).unwrap();
        let entry = Entry { path: "sub/a.txt".to_string(), hash, mode: 0o644 };

        restore_entry(&store_dir, dir.path(), &entry).unwrap();

        assert_eq!(fs::read(dir.path().join("sub").join("a.txt")).unwrap(), b"hello restore");
    }

    #[test]
    fn restore_entry_overwrites_an_existing_file() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        let hash = store::write_blob(&store_dir, &mut &b"new content"[..]).unwrap();
        let entry = Entry { path: "a.txt".to_string(), hash, mode: 0o644 };
        fs::write(dir.path().join("a.txt"), b"old content").unwrap();

        restore_entry(&store_dir, dir.path(), &entry).unwrap();

        assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"new content");
    }

    #[test]
    fn restore_entry_streams_a_large_blob_without_reading_it_all_into_a_buffer_first() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        let content = vec![0x42u8; crate::core::hash::CHUNK_SIZE * 5 + 7];
        let hash = store::write_blob(&store_dir, &mut &content[..]).unwrap();
        let entry = Entry { path: "big.bin".to_string(), hash, mode: 0o644 };

        restore_entry(&store_dir, dir.path(), &entry).unwrap();

        assert_eq!(fs::read(dir.path().join("big.bin")).unwrap(), content);
    }

    #[test]
    fn mirror_restore_writes_target_entries_and_deletes_untracked_extras() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        let hash = store::write_blob(&store_dir, &mut &b"kept"[..]).unwrap();
        let entries = vec![Entry { path: "keep.txt".to_string(), hash, mode: 0o644 }];
        fs::write(dir.path().join("extra.txt"), b"stale").unwrap();

        let stats = mirror_restore(&store_dir, dir.path(), &entries).unwrap();

        assert_eq!(stats.written, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(fs::read(dir.path().join("keep.txt")).unwrap(), b"kept");
        assert!(!dir.path().join("extra.txt").exists());
    }

    #[test]
    fn mirror_restore_to_an_empty_entry_set_deletes_everything_tracked() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();
        fs::write(dir.path().join("b.txt"), b"b").unwrap();

        let stats = mirror_restore(&store_dir, dir.path(), &[]).unwrap();

        assert_eq!(stats.written, 0);
        assert_eq!(stats.deleted, 2);
        assert!(!dir.path().join("a.txt").exists());
        assert!(!dir.path().join("b.txt").exists());
    }

    #[test]
    fn mirror_restore_never_touches_ignored_paths() {
        let dir = tempdir().unwrap();
        let store_dir = store::init_store(dir.path()).unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules").join("dep.js"), b"ignored").unwrap();

        let stats = mirror_restore(&store_dir, dir.path(), &[]).unwrap();

        assert_eq!(stats.deleted, 0, "ignored paths must never be counted as deleted");
        assert_eq!(fs::read(dir.path().join("node_modules").join("dep.js")).unwrap(), b"ignored");
    }
}
