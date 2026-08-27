//! `historia init [dir]` - create a new store (CLAUDE.md §5 init targeting, §9
//! on-disk layout).

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::core::fsutil::display_path;
use crate::core::store::{self, InitError};

pub fn run(args: &[String]) -> Result<(), String> {
    let target = resolve_target(args)?;

    match store::init_store(&target) {
        Ok(store_dir) => {
            println!("Initialized empty historia store in {}", display_path(&store_dir));
            Ok(())
        }
        Err(InitError::AlreadyExists) => Err(format!(
            "historia store already exists at {}",
            display_path(&target.join(store::STORE_DIR_NAME))
        )),
        Err(InitError::Io(e)) => Err(format!("historia init: failed to create store: {e}")),
    }
}

/// Resolve the four `init` targeting forms from CLAUDE.md §5 to an absolute path,
/// creating the target directory (recursively) if it does not exist yet:
/// - `init`        -> current directory
/// - `init .`      -> current directory (explicit)
/// - `init ..`     -> parent directory
/// - `init <path>` -> that path
fn resolve_target(args: &[String]) -> Result<PathBuf, String> {
    let cwd = env::current_dir()
        .map_err(|e| format!("historia init: cannot read current directory: {e}"))?;

    let requested = match args.first().map(String::as_str) {
        None | Some(".") => cwd,
        Some(other) => cwd.join(other),
    };

    fs::create_dir_all(&requested)
        .map_err(|e| format!("historia init: cannot create '{}': {e}", requested.display()))?;

    requested
        .canonicalize()
        .map_err(|e| format!("historia init: cannot resolve '{}': {e}", requested.display()))
}
