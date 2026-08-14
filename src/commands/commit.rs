//! `historia commit -m "msg"` (alias: `snapshot`) - snapshot the whole folder.

use std::env;
use std::path::Path;

use crate::core::store::lock;
use crate::core::{snapshot, store, walk};
use crate::format::manifest::{self, Entry, Manifest};

/// Parsed `commit` arguments: `-m`/`--message <MSG>` and `--allow-empty` (§5).
/// There is deliberately no `add`/staging (§5's non-features) - `commit` always
/// captures the whole tracked folder.
#[derive(Debug, Default, PartialEq, Eq)]
struct CommitArgs {
    message: Option<String>,
    allow_empty: bool,
}

fn parse_args(args: &[String]) -> Result<CommitArgs, String> {
    let mut parsed = CommitArgs::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-m" | "--message" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "historia commit: -m/--message requires a value".to_string())?;
                parsed.message = Some(value.clone());
            }
            "--allow-empty" => parsed.allow_empty = true,
            other => return Err(format!("historia commit: unrecognized argument '{other}'")),
        }
    }
    Ok(parsed)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;
    let message = parsed
        .message
        .ok_or_else(|| "historia commit: -m <message> is required".to_string())?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia commit: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia commit: not a historia store (run 'historia init' first)".to_string()
    })?;

    let guard = lock::acquire(&store_dir).map_err(|e| format!("historia commit: {e}"))?;
    let result = do_commit(&store_dir, &message, parsed.allow_empty);
    guard
        .release()
        .map_err(|e| format!("historia commit: failed to release lock: {e}"))?;

    result.map(|_number| ())
}

/// Do the actual commit work (walk, hash, write blobs, write manifest, advance
/// HEAD - or skip if unchanged), without acquiring the lock itself. Returns the
/// snapshot number the store ends up at (the newly created one, or the current
/// `HEAD` if skip-if-unchanged fired). `pub(super)` so `restore` (CP6) can reuse
/// this exact path for its mandatory pre-restore safety snapshot (Rule 3, forced
/// via `allow_empty: true` so it is never skipped) without re-acquiring the lock
/// `restore` already holds for the whole operation.
pub(super) fn do_commit(store_dir: &Path, message: &str, allow_empty: bool) -> Result<u64, String> {
    // The tracked folder is `.historia`'s parent, not necessarily the current
    // directory - commit always captures the whole folder, wherever it's run
    // from within it (CLAUDE.md §5).
    let root = store_dir
        .parent()
        .expect(".historia is always created inside the tracked folder");

    let walked = walk::walk(root)
        .map_err(|e| format!("historia commit: failed to walk '{}': {e}", root.display()))?;

    for link in &walked.skipped_symlinks {
        println!("skipped symlink: {}", link.display());
    }

    let mut entries = Vec::with_capacity(walked.files.len());
    for file in &walked.files {
        let hash = store::write_blob(store_dir, &mut open_for_hashing(&file.absolute_path)?)
            .map_err(|e| format!("historia commit: failed to store '{}': {e}", file.relative_path))?;
        entries.push(Entry { path: file.relative_path.clone(), hash, mode: file.mode });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let head = snapshot::read_head(store_dir).map_err(|e| format!("historia commit: {e}"))?;
    let head_manifest = snapshot::read_head_manifest(store_dir).map_err(|e| format!("historia commit: {e}"))?;
    let head_entries: &[Entry] = head_manifest.as_ref().map(|m| m.entries.as_slice()).unwrap_or(&[]);

    if !allow_empty && snapshot::entries_match(&entries, head_entries) {
        println!("nothing to snapshot, working folder matches snapshot {head}");
        return Ok(head);
    }

    let number = head + 1;
    let file_count = entries.len();
    let new_manifest = Manifest {
        number,
        timestamp: manifest::now_iso8601_utc(),
        message: message.to_string(),
        parent: head,
        entries,
    };

    // Rule 5's fixed write order: blobs (already written above) -> manifest -> HEAD.
    snapshot::write_manifest(store_dir, &new_manifest).map_err(|e| format!("historia commit: {e}"))?;
    snapshot::write_head(store_dir, number).map_err(|e| format!("historia commit: {e}"))?;

    println!("snapshot {number}: {file_count} file(s) - {message}");
    Ok(number)
}

fn open_for_hashing(path: &Path) -> Result<std::fs::File, String> {
    std::fs::File::open(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_flag_short_form() {
        let args = parse_args(&["-m".to_string(), "hello".to_string()]).unwrap();
        assert_eq!(args.message.as_deref(), Some("hello"));
        assert!(!args.allow_empty);
    }

    #[test]
    fn message_flag_long_form() {
        let args = parse_args(&["--message".to_string(), "hello".to_string()]).unwrap();
        assert_eq!(args.message.as_deref(), Some("hello"));
    }

    #[test]
    fn allow_empty_flag() {
        let args = parse_args(&[
            "-m".to_string(),
            "hello".to_string(),
            "--allow-empty".to_string(),
        ])
        .unwrap();
        assert!(args.allow_empty);
    }

    #[test]
    fn missing_message_value_is_an_error() {
        assert!(parse_args(&["-m".to_string()]).is_err());
    }

    #[test]
    fn no_message_at_all_parses_but_leaves_message_none() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.message, None);
    }

    #[test]
    fn unrecognized_flag_is_an_error() {
        assert!(parse_args(&["--bogus".to_string()]).is_err());
    }
}
