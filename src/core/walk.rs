//! Folder traversal for `commit`/`status`, respecting ignore rules. Symlinks are
//! never followed or stored; each one is reported to the caller so it can print
//! `skipped symlink: <path>` instead of silently dropping it (CLAUDE.md §5).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::ignore;

/// One tracked regular file discovered by [`walk`]: its path relative to the
/// walked root (forward-slash normalized, so manifests are portable across OSes -
/// CLAUDE.md §9), its absolute path (to open/hash/blob it), and its best-effort
/// executable mode (Rule 7).
#[derive(Debug, Clone)]
pub struct WalkedFile {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub mode: u32,
}

/// The result of walking a folder: every tracked regular file, plus every
/// symlink that was found and deliberately skipped (never followed or stored -
/// CLAUDE.md §5).
#[derive(Debug, Default)]
pub struct WalkResult {
    pub files: Vec<WalkedFile>,
    pub skipped_symlinks: Vec<PathBuf>,
}

/// Walk `root`, applying the hardcoded default ignores (`core::ignore`) and
/// always skipping the store directory, wherever either appears in the tree.
pub fn walk(root: &Path) -> io::Result<WalkResult> {
    let mut result = WalkResult::default();
    walk_dir(root, root, &mut result)?;
    result.files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(result)
}

fn walk_dir(root: &Path, dir: &Path, result: &mut WalkResult) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if ignore::is_ignored_name(&name) {
            continue;
        }

        let file_type = entry.file_type()?;
        let path = entry.path();

        if file_type.is_symlink() {
            result.skipped_symlinks.push(path);
        } else if file_type.is_dir() {
            walk_dir(root, &path, result)?;
        } else if file_type.is_file() {
            let relative_path = relative_forward_slash(root, &path)?;
            let mode = mode_for(&entry.metadata()?);
            result.files.push(WalkedFile { relative_path, absolute_path: path, mode });
        }
        // Anything else (device files, sockets, ...) is out of scope for a
        // personal folder backup tool - silently skipped, like a symlink but
        // without a warning since it's not a "lost file" the user would expect.
    }
    Ok(())
}

fn relative_forward_slash(root: &Path, path: &Path) -> io::Result<String> {
    let rel = path.strip_prefix(root).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "walked path was not under the walk root")
    })?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

#[cfg(unix)]
fn mode_for(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 != 0 {
        0o755
    } else {
        0o644
    }
}

/// Windows has no POSIX exec bit (Rule 7) - a sensible, constant default.
#[cfg(not(unix))]
fn mode_for(_metadata: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn finds_files_at_root_and_nested() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("b.txt"), b"b").unwrap();

        let result = walk(dir.path()).unwrap();

        let mut paths: Vec<&str> = result.files.iter().map(|f| f.relative_path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec!["a.txt", "sub/b.txt"]);
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a").join("b")).unwrap();
        fs::write(dir.path().join("a").join("b").join("c.txt"), b"x").unwrap();

        let result = walk(dir.path()).unwrap();

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, "a/b/c.txt");
        assert!(!result.files[0].relative_path.contains('\\'));
    }

    #[test]
    fn default_ignored_directories_are_excluded_wherever_they_appear() {
        let dir = tempdir().unwrap();
        for ignored in [".git", "node_modules", "target", "dist", "build"] {
            let p = dir.path().join(ignored);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("inside.txt"), b"should not appear").unwrap();
        }
        fs::create_dir_all(dir.path().join("sub").join("node_modules")).unwrap();
        fs::write(dir.path().join("sub").join("node_modules").join("nested.txt"), b"x").unwrap();
        fs::write(dir.path().join("keep.txt"), b"keep").unwrap();

        let result = walk(dir.path()).unwrap();

        let paths: Vec<&str> = result.files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["keep.txt"]);
    }

    #[test]
    fn the_store_directory_is_never_walked() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".historia").join("objects")).unwrap();
        fs::write(dir.path().join(".historia").join("HEAD"), b"0\n").unwrap();
        fs::write(dir.path().join("keep.txt"), b"keep").unwrap();

        let result = walk(dir.path()).unwrap();

        let paths: Vec<&str> = result.files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["keep.txt"]);
    }

    #[test]
    fn mode_is_a_sensible_platform_default() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();

        let result = walk(dir.path()).unwrap();

        // Best-effort/platform-dependent (Rule 7): just assert it's populated with
        // one of the two sensible Unix-style modes this crate ever produces.
        assert!(matches!(result.files[0].mode, 0o644 | 0o755));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped_not_followed_and_reported() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("target.txt"), b"real file").unwrap();
        let link = dir.path().join("link.txt");
        symlink(dir.path().join("target.txt"), &link).unwrap();

        let result = walk(dir.path()).unwrap();

        let paths: Vec<&str> = result.files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["target.txt"], "the symlink itself must not be tracked");
        assert_eq!(result.skipped_symlinks, vec![link]);
    }

    #[cfg(windows)]
    #[test]
    fn symlinks_are_skipped_not_followed_and_reported() {
        use std::os::windows::fs::symlink_file;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("target.txt"), b"real file").unwrap();
        let link = dir.path().join("link.txt");
        if symlink_file(dir.path().join("target.txt"), &link).is_err() {
            eprintln!(
                "skipping symlinks_are_skipped_not_followed_and_reported: this environment \
                 cannot create symlinks (needs admin or Developer Mode on Windows)"
            );
            return;
        }

        let result = walk(dir.path()).unwrap();

        let paths: Vec<&str> = result.files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["target.txt"], "the symlink itself must not be tracked");
        assert_eq!(result.skipped_symlinks, vec![link]);
    }
}
