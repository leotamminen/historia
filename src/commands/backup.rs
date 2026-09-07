//! `historia backup <path> [--encrypt] [--force]` - copy the whole store to
//! another local path (CLAUDE.md CP11), or encrypt it into a single opaque
//! file with `--encrypt` (CP15). `historia backup --decrypt <file>
//! <destination> [--force]` reverses the encrypted direction. Read-only on
//! the source store in both directions: never writes to it, never takes its
//! lock (`--decrypt` reads its identity key from the CURRENT store but
//! doesn't modify it either).
//!
//! Destination semantics, kept consistent across all three modes: a plain or
//! decrypted backup's destination is a FOLDER, and the store lands at
//! `<destination>/.historia` (CLAUDE.md CP11) - `<destination>` becomes an
//! ordinary tracked-folder root. An `--encrypt`ed backup's destination is
//! instead a single FILE (CLAUDE.md CP15) - there is no `.historia` involved
//! on that side at all, since the whole point is one opaque blob safe to push
//! to untrusted storage.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::fsutil::display_path;
use crate::core::store::{self, BackupError};
use crate::core::encryption::{self, DecryptBackupError, EncryptBackupError};

#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// `historia backup <dest> [--encrypt] [--force]`.
    Backup { dest: PathBuf, encrypt: bool },
    /// `historia backup --decrypt <source_file> <dest> [--force]`.
    Decrypt { source_file: PathBuf, dest: PathBuf },
}

#[derive(Debug, PartialEq, Eq)]
struct BackupArgs {
    action: Action,
    force: bool,
}

fn parse_args(args: &[String]) -> Result<BackupArgs, String> {
    let mut force = false;
    let mut encrypt = false;
    let mut decrypt_source: Option<PathBuf> = None;
    let mut positional: Option<PathBuf> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--force" => force = true,
            "--encrypt" => encrypt = true,
            "--decrypt" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "historia backup: --decrypt requires a file path".to_string())?;
                decrypt_source = Some(PathBuf::from(value));
            }
            other if positional.is_none() => positional = Some(PathBuf::from(other)),
            other => return Err(format!("historia backup: unexpected extra argument '{other}'")),
        }
    }

    match (decrypt_source, positional) {
        (Some(source_file), Some(dest)) => {
            if encrypt {
                return Err("historia backup: --encrypt and --decrypt cannot be used together".to_string());
            }
            Ok(BackupArgs { action: Action::Decrypt { source_file, dest }, force })
        }
        (Some(_), None) => Err(
            "historia backup: --decrypt requires a destination path \
             (usage: historia backup --decrypt <file> <destination>)"
                .to_string(),
        ),
        (None, Some(dest)) => Ok(BackupArgs { action: Action::Backup { dest, encrypt }, force }),
        (None, None) => Err(
            "historia backup: usage: historia backup <path> [--encrypt] [--force]  |  \
             historia backup --decrypt <file> <destination> [--force]"
                .to_string(),
        ),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia backup: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia backup: not a historia store (run 'historia init' first)".to_string()
    })?;

    match parsed.action {
        Action::Backup { dest, encrypt: false } => run_plain_backup(&store_dir, &dest, parsed.force),
        Action::Backup { dest, encrypt: true } => run_encrypted_backup(&store_dir, &dest, parsed.force),
        Action::Decrypt { source_file, dest } => run_decrypt(&store_dir, &source_file, &dest, parsed.force),
    }
}

fn run_plain_backup(store_dir: &Path, dest: &Path, force: bool) -> Result<(), String> {
    let dest_root = resolve_existing_dir(dest)?;

    match store::backup_store(store_dir, &dest_root, force) {
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

fn run_encrypted_backup(store_dir: &Path, dest_file: &Path, force: bool) -> Result<(), String> {
    let dest_file_abs = resolve_dest_file(dest_file)?;

    match encryption::encrypt_backup(store_dir, &dest_file_abs, force) {
        Ok(counts) => {
            println!(
                "encrypted backup: {} object(s), {} snapshot(s) written to {}",
                counts.objects,
                counts.snapshots,
                display_path(&dest_file_abs)
            );
            Ok(())
        }
        Err(EncryptBackupError::DestinationAlreadyExists) => Err(format!(
            "historia backup: {} already exists (use --force to overwrite)",
            display_path(&dest_file_abs)
        )),
        Err(EncryptBackupError::Key(e)) => Err(format!("historia backup: {e}")),
        Err(EncryptBackupError::Age(e)) => Err(format!("historia backup: encryption failed: {e}")),
        Err(EncryptBackupError::Io(e)) => Err(format!("historia backup: {e}")),
    }
}

fn run_decrypt(store_dir: &Path, source_file: &Path, dest: &Path, force: bool) -> Result<(), String> {
    let dest_root = resolve_existing_dir(dest)?;

    match encryption::decrypt_backup(store_dir, source_file, &dest_root, force) {
        Ok(counts) => {
            println!(
                "decrypted backup: {} object(s), {} snapshot(s) restored to {}",
                counts.objects,
                counts.snapshots,
                display_path(&dest_root.join(store::STORE_DIR_NAME))
            );
            Ok(())
        }
        Err(DecryptBackupError::DestinationAlreadyExists) => Err(format!(
            "historia backup --decrypt: a store already exists at {} (use --force to overwrite)",
            display_path(&dest_root.join(store::STORE_DIR_NAME))
        )),
        Err(DecryptBackupError::Key(e)) => Err(format!("historia backup --decrypt: {e}")),
        Err(DecryptBackupError::Decrypt(e)) => {
            Err(format!("historia backup --decrypt: decryption failed ({e}) - wrong key, or not a valid encrypted historia backup"))
        }
        Err(DecryptBackupError::InvalidArchive(e)) => Err(format!("historia backup --decrypt: {e}")),
        Err(DecryptBackupError::Io(e)) => Err(format!("historia backup --decrypt: {e}")),
    }
}

/// Resolve a destination FOLDER to an absolute path, creating it (recursively)
/// if it doesn't exist yet - mirroring `init`'s own target resolution (CP1).
/// Shared by the plain-backup and decrypt destinations (both land at
/// `<path>/.historia`).
fn resolve_existing_dir(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|e| format!("historia backup: cannot create '{}': {e}", path.display()))?;
    path.canonicalize()
        .map_err(|e| format!("historia backup: cannot resolve '{}': {e}", path.display()))
}

/// Resolve a destination FILE (CP15's `--encrypt` target) to an absolute
/// path. The file itself must NOT already need to exist (that's the whole
/// point - we're about to create it), so this creates and canonicalizes its
/// PARENT directory instead, then re-attaches the file name.
fn resolve_dest_file(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("historia backup: '{}' has no file name", path.display()))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(|e| format!("historia backup: cannot create '{}': {e}", parent.display()))?;
    let abs_parent = parent
        .canonicalize()
        .map_err(|e| format!("historia backup: cannot resolve '{}': {e}", parent.display()))?;
    Ok(abs_parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only() {
        let args = parse_args(&["/tmp/backup".to_string()]).unwrap();
        assert_eq!(args.action, Action::Backup { dest: PathBuf::from("/tmp/backup"), encrypt: false });
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
        assert_eq!(args.action, Action::Backup { dest: PathBuf::from("/tmp/backup"), encrypt: false });
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

    #[test]
    fn encrypt_flag_is_recognized() {
        let args = parse_args(&["/tmp/backup.age".to_string(), "--encrypt".to_string()]).unwrap();
        assert_eq!(args.action, Action::Backup { dest: PathBuf::from("/tmp/backup.age"), encrypt: true });
    }

    #[test]
    fn encrypt_flag_before_path_also_works() {
        let args = parse_args(&["--encrypt".to_string(), "/tmp/backup.age".to_string()]).unwrap();
        assert_eq!(args.action, Action::Backup { dest: PathBuf::from("/tmp/backup.age"), encrypt: true });
    }

    #[test]
    fn decrypt_flag_with_file_and_destination() {
        let args = parse_args(&[
            "--decrypt".to_string(),
            "/tmp/backup.age".to_string(),
            "/tmp/restored".to_string(),
        ])
        .unwrap();
        assert_eq!(
            args.action,
            Action::Decrypt {
                source_file: PathBuf::from("/tmp/backup.age"),
                dest: PathBuf::from("/tmp/restored"),
            }
        );
        assert!(!args.force);
    }

    #[test]
    fn decrypt_with_force() {
        let args = parse_args(&[
            "--decrypt".to_string(),
            "/tmp/backup.age".to_string(),
            "/tmp/restored".to_string(),
            "--force".to_string(),
        ])
        .unwrap();
        assert!(args.force);
    }

    #[test]
    fn decrypt_missing_file_argument_is_an_error() {
        assert!(parse_args(&["--decrypt".to_string()]).is_err());
    }

    #[test]
    fn decrypt_missing_destination_is_an_error() {
        assert!(parse_args(&["--decrypt".to_string(), "/tmp/backup.age".to_string()]).is_err());
    }

    #[test]
    fn decrypt_and_encrypt_together_is_an_error() {
        assert!(parse_args(&[
            "--decrypt".to_string(),
            "/tmp/backup.age".to_string(),
            "/tmp/restored".to_string(),
            "--encrypt".to_string(),
        ])
        .is_err());
    }
}
