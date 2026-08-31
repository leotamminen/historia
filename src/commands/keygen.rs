//! `historia keygen [--force]` - explicitly (re)generate the store's Ed25519
//! signing keypair (CLAUDE.md CP14). The common case (first signed commit)
//! generates a key automatically; this command is for the explicit
//! "regenerate/rotate" case, gated by `--force` once a key already exists -
//! same data-safety pattern as `init`/`backup`'s already-exists refusal.

use std::env;
use std::path::Path;

use crate::core::fsutil::display_path;
use crate::core::signing;
use crate::core::store::{self, lock};

fn parse_args(args: &[String]) -> Result<bool, String> {
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            other => return Err(format!("historia keygen: unrecognized argument '{other}'")),
        }
    }
    Ok(force)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let force = parse_args(args)?;

    let cwd = env::current_dir()
        .map_err(|e| format!("historia keygen: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia keygen: not a historia store (run 'historia init' first)".to_string()
    })?;

    let guard = lock::acquire(&store_dir).map_err(|e| format!("historia keygen: {e}"))?;
    let result = do_keygen(&store_dir, force);
    guard
        .release()
        .map_err(|e| format!("historia keygen: failed to release lock: {e}"))?;

    result
}

fn do_keygen(store_dir: &Path, force: bool) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_means_no_force() {
        let force = parse_args(&[]).unwrap();
        assert!(!force);
    }

    #[test]
    fn force_flag_is_recognized() {
        let force = parse_args(&["--force".to_string()]).unwrap();
        assert!(force);
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
