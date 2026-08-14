//! `historia restore <n> [path]` - restore the folder, or one file, to a past
//! snapshot (exact mirror of the tracked set, safety snapshot first, CLAUDE.md
//! Rules 3, 4, 5, 6, 7).

use std::env;
use std::path::Path;

use crate::core::store::lock;
use crate::core::{fsutil, snapshot, store};
use crate::format::manifest::Manifest;

#[derive(Debug, PartialEq, Eq)]
struct RestoreArgs {
    number: u64,
    path: Option<String>,
}

fn parse_args(args: &[String]) -> Result<RestoreArgs, String> {
    let number_str = args
        .first()
        .ok_or_else(|| "historia restore: usage: historia restore <n> [path]".to_string())?;
    let number: u64 = number_str
        .parse()
        .map_err(|_| format!("historia restore: '{number_str}' is not a valid snapshot number"))?;

    if args.len() > 2 {
        return Err(format!("historia restore: unexpected extra argument '{}'", args[2]));
    }
    // Manifest paths are always forward-slash normalized (CLAUDE.md §9); accept
    // a Windows-style path from the user and normalize it the same way so it
    // matches.
    let path = args.get(1).map(|p| p.replace('\\', "/"));

    Ok(RestoreArgs { number, path })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia restore: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia restore: not a historia store (run 'historia init' first)".to_string()
    })?;

    let guard = lock::acquire(&store_dir).map_err(|e| format!("historia restore: {e}"))?;
    let result = do_restore(&store_dir, parsed.number, parsed.path.as_deref());
    guard
        .release()
        .map_err(|e| format!("historia restore: failed to release lock: {e}"))?;

    result
}

fn do_restore(store_dir: &Path, number: u64, path: Option<&str>) -> Result<(), String> {
    // Validate the target snapshot exists before mutating anything at all - not
    // even the safety snapshot - so a bad snapshot number is a pure no-op.
    let target = snapshot::read_manifest(store_dir, number)
        .map_err(|_| format!("historia restore: snapshot {number} does not exist"))?;

    // Rule 3: the safety snapshot happens unconditionally, before any working
    // folder mutation, reusing CP3's commit path exactly. Forced via
    // `allow_empty: true` so it is never skipped even if the working folder
    // currently matches HEAD - the pre-restore state must always be recoverable.
    let safety_message = format!("auto: safety snapshot before restore to {number}");
    let safety_number = crate::commands::commit::do_commit(store_dir, &safety_message, true)?;

    match path {
        Some(single_path) => restore_single_file(store_dir, &target, single_path, safety_number),
        None => restore_whole_folder(store_dir, &target, safety_number),
    }
}

fn restore_whole_folder(store_dir: &Path, target: &Manifest, safety_number: u64) -> Result<(), String> {
    let root = tracked_root(store_dir);
    let stats = fsutil::mirror_restore(store_dir, root, &target.entries)
        .map_err(|e| format!("historia restore: failed to restore: {e}"))?;

    println!(
        "restored to snapshot {}: {} file(s) written, {} file(s) deleted (safety snapshot {} created first)",
        target.number, stats.written, stats.deleted, safety_number
    );
    Ok(())
}

fn restore_single_file(
    store_dir: &Path,
    target: &Manifest,
    path: &str,
    safety_number: u64,
) -> Result<(), String> {
    let entry = target.entries.iter().find(|e| e.path == path).ok_or_else(|| {
        format!(
            "historia restore: '{path}' is not in snapshot {} (safety snapshot {} was still created)",
            target.number, safety_number
        )
    })?;

    let root = tracked_root(store_dir);
    fsutil::restore_entry(store_dir, root, entry)
        .map_err(|e| format!("historia restore: failed to restore '{path}': {e}"))?;

    println!(
        "restored '{path}' from snapshot {} (safety snapshot {} created first)",
        target.number, safety_number
    );
    Ok(())
}

fn tracked_root(store_dir: &Path) -> &Path {
    store_dir
        .parent()
        .expect(".historia is always created inside the tracked folder")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_folder_form_parses_the_number_only() {
        let args = parse_args(&["3".to_string()]).unwrap();
        assert_eq!(args.number, 3);
        assert_eq!(args.path, None);
    }

    #[test]
    fn single_file_form_parses_number_and_path() {
        let args = parse_args(&["3".to_string(), "src/main.rs".to_string()]).unwrap();
        assert_eq!(args.number, 3);
        assert_eq!(args.path.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn backslash_paths_are_normalized_to_forward_slashes() {
        let args = parse_args(&["1".to_string(), "sub\\file.txt".to_string()]).unwrap();
        assert_eq!(args.path.as_deref(), Some("sub/file.txt"));
    }

    #[test]
    fn missing_number_is_an_error() {
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn non_numeric_number_is_an_error() {
        assert!(parse_args(&["abc".to_string()]).is_err());
    }

    #[test]
    fn too_many_arguments_is_an_error() {
        assert!(parse_args(&["1".to_string(), "a".to_string(), "b".to_string()]).is_err());
    }
}
