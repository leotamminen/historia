//! `historia keygen [--signing | --encryption] [--force]` - explicitly
//! (re)generate a store's keypair (CLAUDE.md CP14, extended by CP15). The
//! common case (first signed commit, or first `--encrypt` backup) generates
//! whichever key is needed automatically; this command is for the explicit
//! "regenerate/rotate" case, gated by `--force` once a key already exists -
//! same data-safety pattern as `init`/`backup`'s already-exists refusal.
//!
//! Flag scheme (CLAUDE.md CP15: "keep the flag scheme consistent"): `--force`
//! means the same thing for either key ("overwrite despite one existing").
//! `--signing`/`--encryption` choose WHICH key; `--signing` is also the
//! default with neither flag given, so `historia keygen [--force]` keeps
//! meaning exactly what it meant before CP15 introduced a second key type.

use std::env;
use std::path::Path;

use crate::core::fsutil::display_path;
use crate::core::store::{self, lock};
use crate::core::{encryption, signing};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Signing,
    Encryption,
}

#[derive(Debug, PartialEq, Eq)]
struct KeygenArgs {
    kind: KeyKind,
    force: bool,
}

fn parse_args(args: &[String]) -> Result<KeygenArgs, String> {
    let mut kind: Option<KeyKind> = None;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            "--signing" if kind.is_none() => kind = Some(KeyKind::Signing),
            "--encryption" if kind.is_none() => kind = Some(KeyKind::Encryption),
            "--signing" | "--encryption" => {
                return Err("historia keygen: --signing and --encryption cannot be used together".to_string())
            }
            other => return Err(format!("historia keygen: unrecognized argument '{other}'")),
        }
    }
    Ok(KeygenArgs { kind: kind.unwrap_or(KeyKind::Signing), force })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia keygen: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia keygen: not a historia store (run 'historia init' first)".to_string()
    })?;

    let guard = lock::acquire(&store_dir).map_err(|e| format!("historia keygen: {e}"))?;
    let result = do_keygen(&store_dir, parsed.kind, parsed.force);
    guard
        .release()
        .map_err(|e| format!("historia keygen: failed to release lock: {e}"))?;

    result
}

fn do_keygen(store_dir: &Path, kind: KeyKind, force: bool) -> Result<(), String> {
    match kind {
        KeyKind::Signing => do_keygen_signing(store_dir, force),
        KeyKind::Encryption => do_keygen_encryption(store_dir, force),
    }
}

fn do_keygen_signing(store_dir: &Path, force: bool) -> Result<(), String> {
    if signing::any_key_file_present(store_dir) && !force {
        return Err(
            "historia keygen: a signing key already exists; refusing to overwrite it \
             (use --force). Overwriting means this store can no longer produce NEW signatures \
             under the old identity: snapshots already signed remain cryptographically valid \
             under that old key, but since only the CURRENT public key is kept on disk, \
             'historia verify' will no longer be able to confirm them - it will report them as \
             having an invalid signature."
                .to_string(),
        );
    }

    signing::generate_and_save_key(store_dir)
        .map_err(|e| format!("historia keygen: failed to write signing key: {e}"))?;

    println!(
        "generated a new signing key at {}",
        display_path(&store_dir.join(signing::PRIVATE_KEY_FILE_NAME))
    );
    Ok(())
}

fn do_keygen_encryption(store_dir: &Path, force: bool) -> Result<(), String> {
    if encryption::any_key_file_present(store_dir) && !force {
        return Err(
            "historia keygen: an age encryption key already exists; refusing to overwrite it \
             (use --force). Overwriting is sharp: encrypted backups already made under the OLD \
             identity become PERMANENTLY UNDECRYPTABLE once it's gone - only the current key is \
             kept on disk, and there is no way to recover the old one. New encrypted backups \
             going forward will use the new identity just fine."
                .to_string(),
        );
    }

    encryption::generate_and_save_key(store_dir)
        .map_err(|e| format!("historia keygen: failed to write age encryption key: {e}"))?;

    println!(
        "generated a new age encryption key at {}",
        display_path(&store_dir.join(encryption::IDENTITY_FILE_NAME))
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_means_signing_key_and_no_force() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.kind, KeyKind::Signing);
        assert!(!args.force);
    }

    #[test]
    fn signing_flag_is_explicit_and_recognized() {
        let args = parse_args(&["--signing".to_string()]).unwrap();
        assert_eq!(args.kind, KeyKind::Signing);
    }

    #[test]
    fn encryption_flag_selects_the_age_key() {
        let args = parse_args(&["--encryption".to_string()]).unwrap();
        assert_eq!(args.kind, KeyKind::Encryption);
    }

    #[test]
    fn force_flag_is_recognized_alongside_encryption() {
        let args = parse_args(&["--encryption".to_string(), "--force".to_string()]).unwrap();
        assert_eq!(args.kind, KeyKind::Encryption);
        assert!(args.force);
    }

    #[test]
    fn signing_and_encryption_together_is_an_error() {
        assert!(parse_args(&["--signing".to_string(), "--encryption".to_string()]).is_err());
    }

    #[test]
    fn unrecognized_argument_is_an_error() {
        assert!(parse_args(&["--bogus".to_string()]).is_err());
    }

    #[test]
    fn extra_positional_argument_is_an_error() {
        assert!(parse_args(&["unexpected".to_string()]).is_err());
    }
}
