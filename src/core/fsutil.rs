//! Atomic write-then-rename helpers, and the safe (mirror) restore routine that
//! makes the working folder match a snapshot exactly without ever touching ignored
//! paths (CLAUDE.md Rule 3, Rule 4, Rule 5).

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Write `contents` to `path` atomically: write to a sibling temp file, `fsync` it,
/// then rename over the destination. A crash or power loss can never leave `path`
/// holding partial contents - readers see either the old contents or the new ones,
/// never a mix (CLAUDE.md Rule 5).
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
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

    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, path)?;
    Ok(())
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
}
