//! `historia backup <path> [--force]` - copy the whole store to another local
//! path (CLAUDE.md CP11). Read-only on the source: never writes to it, never
//! takes its lock.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::core::fsutil::display_path;
use crate::core::store::{self, BackupError};

#[derive(Debug, Default, PartialEq, Eq)]
struct BackupArgs {
    dest: PathBuf,
    force: bool,
}

fn parse_args(args: &[String]) -> Result<BackupArgs, String> {
    let mut dest: Option<PathBuf> = None;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            other if dest.is_none() => dest = Some(PathBuf::from(other)),
            other => return Err(format!("historia backup: unexpected extra argument '{other}'")),
        }
    }
    let dest = dest.ok_or_else(|| "historia backup: usage: historia backup <path> [--force]".to_string())?;
    Ok(BackupArgs { dest, force })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia backup: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia backup: not a historia store (run 'historia init' first)".to_string()
    })?;

    let dest_root = resolve_dest(&parsed.dest)?;

    match store::backup_store(&store_dir, &dest_root, parsed.force) {
        Ok(stats) => {
            println!(
                "backed up {} object(s), {} snapshot(s) to {}",
                stats.objects_copied,
                stats.snapshots_copied,
                display_path(&dest_root.join(store::STORE_DIR_NAME))
            );
            Ok(())
        }
        Err(BackupError::DestinationAlreadyExists) => Err(format!(
            "historia backup: a store already exists at {} (use --force to overwrite)",
            display_path(&dest_root.join(store::STORE_DIR_NAME))
        )),
        Err(BackupError::Io(e)) => Err(format!("historia backup: failed to copy store: {e}")),
    }
}

/// Resolve the destination path to an absolute one, creating it (recursively)
/// if it doesn't exist yet - mirroring `init`'s own target resolution (CP1).
fn resolve_dest(path: &std::path::Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|e| format!("historia backup: cannot create '{}': {e}", path.display()))?;
    path.canonicalize()
        .map_err(|e| format!("historia backup: cannot resolve '{}': {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only() {
        let args = parse_args(&["/tmp/backup".to_string()]).unwrap();
        assert_eq!(args.dest, PathBuf::from("/tmp/backup"));
        assert!(!args.force);
    }

    #[test]
    fn path_with_force_flag() {
        let args = parse_args(&["/tmp/backup".to_string(), "--force".to_string()]).unwrap();
        assert!(args.force);
    }

    #[test]
    fn force_flag_before_path_also_works() {
        let args = parse_args(&["--force".to_string(), "/tmp/backup".to_string()]).unwrap();
        assert_eq!(args.dest, PathBuf::from("/tmp/backup"));
        assert!(args.force);
    }

    #[test]
    fn missing_path_is_an_error() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--force".to_string()]).is_err());
    }

    #[test]
    fn two_paths_is_an_error() {
        assert!(parse_args(&["a".to_string(), "b".to_string()]).is_err());
    }

    #[test]
    fn unrecognized_flag_is_an_error() {
        assert!(parse_args(&["/tmp/backup".to_string(), "--bogus".to_string()]).is_err());
    }
}
