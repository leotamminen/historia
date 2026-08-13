//! Default ignores (`.git`, `node_modules`, `target`, `dist`, `build`, `.historia/`)
//! layered with `.historiaignore`, matched via the `ignore` crate (CLAUDE.md §5, §7).

/// Names ignored anywhere in the tree, regardless of depth (CLAUDE.md §5).
/// Overridable later via `.historiaignore` (CP7); hardcoded for now since CP3
/// doesn't parse that file yet, and no `ignore` crate dependency until then either.
const DEFAULT_IGNORED_NAMES: &[&str] = &[".git", "node_modules", "target", "dist", "build"];

/// True if a file or directory with this name should never be walked into or
/// tracked, wherever it appears in the tree. `.historia/` itself is always
/// ignored too, and - unlike the rest of this list - stays non-overridable even
/// once `.historiaignore` support lands in CP7 (CLAUDE.md §5).
pub fn is_ignored_name(name: &str) -> bool {
    name == crate::core::store::STORE_DIR_NAME || DEFAULT_IGNORED_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ignored_names_are_excluded() {
        for name in [".git", "node_modules", "target", "dist", "build"] {
            assert!(is_ignored_name(name), "{name} should be ignored by default");
        }
    }

    #[test]
    fn the_store_directory_is_always_ignored() {
        assert!(is_ignored_name(".historia"));
    }

    #[test]
    fn ordinary_names_are_not_ignored() {
        for name in ["src", "main.rs", "README.md", "Cargo.toml"] {
            assert!(!is_ignored_name(name), "{name} should not be ignored");
        }
    }
}
